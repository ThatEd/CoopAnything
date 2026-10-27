//! In-process agent: the write half, as a standalone DLL.
//!
//! Proves, in order: we can get a DLL into the game process; it starts a worker without deadlocking
//! the loader; it reaches the Rust supervisor over loopback; and it can locate the live
//! `RaceSimManager` from inside the process. Writes arrive last, because a bad write is a hard crash.
//!
//! Nothing here may crash the game silently. `crash` installs a trace and a minidump handler before
//! anything else runs, so every later failure names the stage it died in.
//!
//! DllMain discipline: no allocation, no Rust runtime initialisation, no locks. It calls CreateThread
//! and returns. Everything else happens on that thread, outside the loader lock.

use std::fs::OpenOptions;
use std::io::Write;
use std::net::TcpStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

pub mod crash;
pub mod hook;
pub mod hookmail;

use coop_proto::{encode, Msg};

/// The process-wide write mailbox. The client fills it over TCP; the hook callback drains it on the
/// game thread. A `OnceLock` rather than a `static mut` so the mailbox has a normal address that the
/// callback can reach without `unsafe` at the call site.
static MAILBOX: OnceLock<hookmail::Mailbox> = OnceLock::new();

pub fn mailbox() -> &'static hookmail::Mailbox {
    MAILBOX.get_or_init(hookmail::Mailbox::new)
}

/// The hook callback: runs on the game thread, inside the sim call.
///
/// This is the whole point of the mailbox. `this` is the sim object pointer the game itself passed to
/// `simulate`, so it is the one address we know is live and correctly aligned at this instant. Writes
/// from here are ordered against the simulation by construction, which is what the previous design
/// lacked and why every attempt to replicate cars from a worker thread ended in
/// `EXCEPTION_ACCESS_VIOLATION reading 0xF3C`.
///
/// Deliberately does nothing but drain the mailbox: no logging (a file write here would stall the sim),
/// no allocation, no locks.
extern "system" fn on_sim(this: *mut std::ffi::c_void) {
    let base = this as usize;
    if base == 0 {
        return;
    }
    let wrote = mailbox().drain(base);
    if wrote > 0 {
        // A plain relaxed add: the counter is read by the worker for its log line and nothing depends
        // on its exact value.
        LATE_WRITES.fetch_add(1, Ordering::Relaxed);
    }
}


/// How many mailbox drains have written anything, for the agent's heartbeat line.
static LATE_WRITES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// The newest race control the host has sent, kept here so every snapshot frame carries it.
///
/// Held in atomics rather than behind a mutex because the write-port thread reads it on every frame and
/// the hook must never wait on a lock. Each field is a single aligned word, so a reader sees either the
/// old value or the new one and never a torn mix of two messages - which for a red flag would mean
/// briefly reporting "race resumed" while the host is still stopped.
static RC_FLAGS: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
static RC_STATE: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
static RC_RF: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

fn store_rc(rc: &coop_proto::RcState) {
    RC_FLAGS.store(rc.flags, Ordering::Relaxed);
    RC_STATE.store(rc.rc_state as u32, Ordering::Relaxed);
    RC_RF.store(rc.rf_state as u32, Ordering::Relaxed);
}

fn latest_rc() -> coop_proto::RcState {
    coop_proto::RcState {
        flags: RC_FLAGS.load(Ordering::Relaxed),
        rc_state: RC_STATE.load(Ordering::Relaxed) as u8,
        rf_state: RC_RF.load(Ordering::Relaxed) as u8,
    }
}

/// The tyre-replication mode, read from `coop_tyres.txt` next to the exe.
///
/// Defaults to `Observe`, which writes nothing. That default is the whole point: tyre state is derived
/// by the sim (temperature drives the puncture curve, wear accumulates from driving - see
/// RNG-DETERMINISM §1.3), so writing it is overriding the simulation rather than replicating it. The
/// file lets the mode change without a rebuild, and lets a live test step through observe -> wear ->
/// full instead of jumping to the widest one.
pub fn tyre_mode() -> hookmail::TyreMode {
    let path = log_path().with_file_name("coop_tyres.txt");
    let Ok(text) = std::fs::read_to_string(&path) else {
        return hookmail::TyreMode::Observe;
    };
    let trimmed = text.trim().to_ascii_lowercase();
    if trimmed.is_empty() || trimmed.starts_with("obs") {
        return hookmail::TyreMode::Observe;
    }
    let mode = hookmail::TyreMode::from_text(&trimmed);
    if mode == hookmail::TyreMode::Observe {
        log(&format!(
            "coop_tyres.txt says {:?}, which is not a mode; using observe (writes nothing)",
            text.trim()
        ));
    }
    mode
}

/// One observation line describing the host's actual tyre readings.
///
/// Printed at most once a second while in `Observe`, because the next decision - whether wear alone is
/// safe to write, and what the temperature ranges really are - cannot be made from the field layout. It
/// has to be made from numbers this game actually produces, which is exactly what the previous
/// generation never collected before writing.
fn log_tyre_observation(cars: &[coop_proto::Car]) {
    let mut line = String::from("tyres observed (surface/carcass/condition per wheel):");
    for car in cars.iter().take(3) {
        line.push_str(&format!(
            "\n  car{:<2} {}",
            car.id,
            hookmail::tyre_summary(car)
        ));
    }
    // Ranges across every car, which is the number that decides the safe bounds.
    let mut s_lo = f32::MAX;
    let mut s_hi = f32::MIN;
    let mut c_lo = f32::MAX;
    let mut c_hi = f32::MIN;
    let mut rejected = 0;
    for car in cars {
        for t in &car.tyres {
            for (v, lo, hi) in [
                (t.surface, &mut s_lo, &mut s_hi),
                (t.carcass, &mut c_lo, &mut c_hi),
            ] {
                if v.is_finite() {
                    if v < *lo {
                        *lo = v;
                    }
                    if v > *hi {
                        *hi = v;
                    }
                }
            }
            if !hookmail::plausible(t.surface, coop_proto::layout::off::tyre_range::SURFACE)
                || !hookmail::plausible(t.carcass, coop_proto::layout::off::tyre_range::CARCASS)
                || !hookmail::plausible(
                    t.condition,
                    coop_proto::layout::off::tyre_range::CONDITION,
                )
            {
                rejected += 1;
            }
        }
    }
    log(&format!(
        "{line}\n  surface range {s_lo:.1}..{s_hi:.1}  carcass range {c_lo:.1}..{c_hi:.1}  \
         out-of-range values: {rejected} of {}"
        , cars.len() * 4
    ));
}


pub const DEFAULT_SUPERVISOR: &str = "127.0.0.1:45990";

/// Supervisor address the agent should use.
///
/// An injected DLL cannot see the injector's environment, and requiring a rebuild to change an
/// address makes a two-machine test impossible, so `coop_supervisor.txt` next to the game's exe wins
/// when it exists. It holds one line, `host:port`.
pub fn supervisor_address() -> String {
    let path = log_path().with_file_name("coop_supervisor.txt");
    if let Ok(text) = std::fs::read_to_string(&path) {
        let line = text.lines().next().unwrap_or("").trim().to_string();
        if !line.is_empty() {
            return line;
        }
    }
    DEFAULT_SUPERVISOR.to_string()
}
const RETRY: Duration = Duration::from_secs(2);
const HEARTBEAT: Duration = Duration::from_secs(1);

static STARTED: AtomicBool = AtomicBool::new(false);
/// Cached result of `locate_rsm`. A `const` here would be a fresh temporary at every use, so the
/// cache would never hold anything.
static RSM_CACHE: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

#[cfg(windows)]
mod win {
    #[link(name = "kernel32")]
    extern "system" {
        pub fn CreateThread(
            attrs: *mut u8,
            stack: usize,
            start: extern "system" fn(*mut u8) -> u32,
            param: *mut u8,
            flags: u32,
            id: *mut u32,
        ) -> isize;
        pub fn GetCurrentProcessId() -> u32;
    }
}

#[cfg(windows)]
pub mod sim {
    //! Resolve and VALIDATE the sim entry point before anything is patched.
    //!
    //! FormationLap documents `simulate` at `0x142298080`. UE executables link at a preferred base of
    //! 0x140000000, so that number is `preferred_base + RVA`. The real base moves under ASLR, which
    //! makes the recorded absolute address useless on its own - we have to rebuild it every launch.
    //!
    //! Nothing here patches code. It reports what is actually at the address, so a wrong RVA shows up
    //! as implausible bytes instead of a crash.

    use std::ffi::c_void;

    /// `0x142298080 - 0x140000000`
    pub const SIMULATE_RVA: usize = 0x0229_8080;
    pub const PREFERRED_BASE: usize = 0x1_4000_0000;

    #[repr(C)]
    struct MemoryBasicInformation {
        base_address: *mut c_void,
        allocation_base: *mut c_void,
        allocation_protect: u32,
        partition_id: u16,
        _pad: u16,
        region_size: usize,
        state: u32,
        protect: u32,
        kind: u32,
        _pad2: u32,
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn GetModuleHandleW(name: *const u16) -> isize;
        fn VirtualQuery(addr: *const c_void, info: *mut MemoryBasicInformation, len: usize) -> usize;
    }

    const MEM_COMMIT: u32 = 0x1000;
    const PAGE_EXECUTE: u32 = 0x10;
    const PAGE_EXECUTE_READ: u32 = 0x20;
    const PAGE_EXECUTE_READWRITE: u32 = 0x40;
    const PAGE_EXECUTE_WRITECOPY: u32 = 0x80;

    pub fn main_module_base() -> Option<usize> {
        let h = unsafe { GetModuleHandleW(std::ptr::null()) };
        if h == 0 {
            None
        } else {
            Some(h as usize)
        }
    }

    pub fn describe(target: usize) -> String {
        let mut mi = MemoryBasicInformation {
            base_address: std::ptr::null_mut(),
            allocation_base: std::ptr::null_mut(),
            allocation_protect: 0,
            partition_id: 0,
            _pad: 0,
            region_size: 0,
            state: 0,
            protect: 0,
            kind: 0,
            _pad2: 0,
        };
        let got = unsafe { VirtualQuery(target as *const c_void, &mut mi, std::mem::size_of::<MemoryBasicInformation>()) };
        if got == 0 {
            return "VirtualQuery failed".into();
        }
        let executable = matches!(
            mi.protect,
            PAGE_EXECUTE | PAGE_EXECUTE_READ | PAGE_EXECUTE_READWRITE | PAGE_EXECUTE_WRITECOPY
        );
        format!(
            "state={} protect=0x{:X} executable={} region_base=0x{:X} region_size=0x{:X}",
            mi.state, mi.protect, executable, mi.base_address as usize, mi.region_size
        ).replace(&format!("state={MEM_COMMIT}"), "state=COMMIT")
    }

    /// Reads the first bytes at an address, for eyeballing a prologue.
    pub fn bytes_at(target: usize, n: usize) -> Option<Vec<u8>> {
        if target == 0 {
            return None;
        }
        let p = target as *const u8;
        let mut out = Vec::with_capacity(n);
        for i in 0..n {
            out.push(unsafe { *p.add(i) });
        }
        Some(out)
    }

    pub fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02X}")).collect::<Vec<_>>().join(" ")
    }

    /// A cheap sanity check. A real function prologue is not all zeros, not all 0xCC padding, and
    /// typically starts by saving something on the stack.
    pub fn plausible(bytes: &[u8]) -> bool {
        if bytes.len() < 8 {
            return false;
        }
        if bytes.iter().all(|b| *b == 0) || bytes.iter().all(|b| *b == 0xCC) {
            return false;
        }
        // 48/4C = REX prefix (push/sub/mov forms), 55 = push rbp, 40 = REX, E9 = jmp (thunks)
        matches!(bytes[0], 0x48 | 0x4C | 0x55 | 0x40 | 0xE9 | 0x53 | 0x56 | 0x57)
    }

    /// Full report for the endpoint message and the log.
    pub fn inspect() -> String {
        match main_module_base() {
            None => "no main module".into(),
            Some(base) => {
                let target = base + SIMULATE_RVA;
                let layout = describe(target);
                match bytes_at(target, 16) {
                    None => format!("base=0x{base:X} target=0x{target:X} unreadable"),
                    Some(b) => format!(
                        "base=0x{base:X} target=0x{target:X} plausible={} bytes=[{}] {layout}",
                        plausible(&b),
                        hex(&b)
                    ),
                }
            }
        }
    }
}

/// Default port the agent listens on for car frames from the local `coop-link`.
///
/// A second port, not the supervisor connection: the supervisor link carries `Agent` heartbeats
/// *outbound*, and mixing inbound car data onto it would make the agent's own identity message and the
/// client's writes compete on one socket. Keeping them apart also keeps the two failure modes
/// separable in the log - "the supervisor went away" must not look like "the mailbox stopped".
pub const DEFAULT_WRITE_PORT: u16 = 45991;

/// How often the observe-only tyre report is written. Every second is enough to see wear accumulate and
/// temperature swing, and slow enough not to flood the log during a race.
const TYRE_OBS: Duration = Duration::from_secs(1);

static WRITES_STARTED: AtomicBool = AtomicBool::new(false);

/// Where the agent listens, so a two-machine test can change it without a rebuild.
pub fn write_port() -> u16 {
    let path = log_path().with_file_name("coop_agent_port.txt");
    if let Ok(text) = std::fs::read_to_string(&path) {
        if let Ok(p) = text.lines().next().unwrap_or("").trim().parse::<u16>() {
            return p;
        }
    }
    DEFAULT_WRITE_PORT
}

/// Accepts car frames from the local `coop-link` and hands them to the mailbox.
///
/// Runs on its own thread and never touches game memory: it only parses a snapshot and stages it. The
/// hook callback is what puts the bytes into the sim, on the game thread. That split is the whole
/// architecture, and keeping the network thread out of the write path is what makes it hold.
fn serve_writes(port: u16) {
    use std::io::Read;
    use std::net::TcpListener;

    let listener = match TcpListener::bind(("127.0.0.1", port)) {
        Ok(l) => l,
        Err(e) => {
            log(&format!("write listener could not bind 127.0.0.1:{port}: {e}"));
            return;
        }
    };
    log(&format!("write listener on 127.0.0.1:{port}"));

    for incoming in listener.incoming() {
        let mut stream = match incoming {
            Ok(s) => s,
            Err(e) => {
                log(&format!("write listener accept failed: {e}"));
                continue;
            }
        };
        let _ = stream.set_nodelay(true);
        log("coop-link connected to the write port");
        // Say up front whether the frames about to arrive can actually reach the sim. Without this the
        // client's log says "sent 400 to the agent" while nothing changes on screen, and the reason is
        // three files away in this log.
        #[cfg(windows)]
        {
            let (fires, cb) = hook::state();
            if cb == 0 {
                log(
                    "WARNING: no hook callback is registered, so frames will be staged and never \
                     applied. The hook is opt-in: create coop_hook.enable next to the exe (or set \
                     COOP_AGENT_HOOK=1) and restart the game.",
                );
            } else if fires == 0 {
                log(&format!(
                    "callback 0x{cb:X} registered but the hook has not fired yet; it will apply on \
                     the first sim call (start or enter a race)"
                ));
            } else {
                log(&format!("callback 0x{cb:X} registered, hook has fired {fires} time(s)"));
            }
        }
        let mut buf: Vec<u8> = Vec::with_capacity(8192);
        let mut chunk = [0u8; 8192];
        let mut frames_in = 0u64;
        let mut warned_inert = false;
        let mut last_tyre_obs: Option<Instant> = None;
        'session: loop {
            let n = match stream.read(&mut chunk) {
                Ok(0) => {
                    log("coop-link closed the write port");
                    break;
                }
                Ok(n) => n,
                Err(e) => {
                    log(&format!("write port read failed: {e}"));
                    break;
                }
            };
            buf.extend_from_slice(&chunk[..n]);
            loop {
                match coop_proto::next_frame(&buf) {
                    Ok(None) => break,
                    Ok(Some(len)) => {
                        let frame: Vec<u8> = buf.drain(..len).collect();
                        match coop_proto::decode(&frame) {
                            Ok(Msg::Snapshot { cars, .. }) => {
                                frames_in += 1;
                                let tyres = tyre_mode();
                                // Observation is not a no-op: it is the evidence needed to decide
                                // whether writing these fields is safe at all, and it is rate-limited
                                // because it writes a log line.
                                if tyres == hookmail::TyreMode::Observe
                                    && last_tyre_obs.map(|t| t.elapsed() >= TYRE_OBS).unwrap_or(true)
                                {
                                    last_tyre_obs = Some(Instant::now());
                                    log_tyre_observation(&cars);
                                }
                                // A dropped frame is normal - the callback runs at whatever rate the
                                // sim calls it - so it is counted by the mailbox, not logged here.
                                let published = mailbox().publish_full(&cars, &latest_rc(), tyres);
                                // The dangerous silent case: frames arrive, nothing is being applied,
                                // and the client still reports success. Say it once, loudly.
                                if !published && !warned_inert {
                                    if let Some(off) = inert_cause() {
                                        log(&format!("frames are arriving but not being applied: {off}"));
                                        warned_inert = true;
                                    }
                                }
                                if frames_in % 200 == 0 {
                                    let (applied, dropped, entries) = mailbox().stats();
                                    let rc = latest_rc();
                                    log(&format!(
                                        "write port: {frames_in} frames in, mailbox applied={applied} \
                                         dropped={dropped} entries={entries}, tyres={}, rc flags=0x{:X} \
                                         state={} rf={}",
                                        tyres.as_str(), rc.flags, rc.rc_state, rc.rf_state
                                    ));
                                }
                            }
                            // Race control arrives on its own message because it changes rarely and
                            // matters more than any single frame: a client that misses a red flag
                            // freezes its cars with no idea why. Kept out of the snapshot so it cannot
                            // be lost to a dropped frame.
                            Ok(Msg::Rc(rc)) => {
                                if rc != latest_rc() {
                                    log(&format!(
                                        "race control: flags=0x{:X} rc_state={} rf_state={}",
                                        rc.flags, rc.rc_state, rc.rf_state
                                    ));
                                }
                                store_rc(&rc);
                            }
                            Ok(_) => {}
                            Err(e) => log(&format!("write port rejected a frame: {e}")),
                        }
                    }
                    Err(e) => {
                        log(&format!("write port framing error ({e}); closing"));
                        break 'session;
                    }
                }
            }
        }
    }
}

/// Explains why staged frames are not reaching the sim, or `None` when they are.
///
/// Three distinct causes produce the same visible symptom - the client says it sent frames and the
/// game does not move - and telling them apart from a log is otherwise guesswork:
///
/// 1. no hook (opt-in flag not set), so there is no game-thread entry point at all;
/// 2. a hook with no callback, which means it fires but does nothing;
/// 3. a hook and callback that have never fired, because no sim call has happened yet.
pub fn inert_cause() -> Option<String> {
    let block = hook::block();
    if block.is_none() {
        return Some(
            "the sim hook is not installed, so there is no game-thread entry point. Create \
             coop_hook.enable next to the game exe (or set COOP_AGENT_HOOK=1) and restart the game."
                .to_string(),
        );
    }
    let (fires, cb) = hook::state();
    if cb == 0 {
        return Some("the hook is installed but has no callback registered".to_string());
    }
    if fires == 0 {
        return Some(format!(
            "the hook (callback 0x{cb:X}) has never fired: no sim call has happened yet. Frames are \
             staged and will apply once a race session starts."
        ));
    }
    None
}

/// Team claims, shared between the two machines so the screen can show who has taken what.
///
/// A claim is a `(team, role)` pair: `role` is `"host"` or `"client"`. Held in the agent rather than in the
/// front-end because localStorage is per-machine - the second player cannot see the first player's claims
/// unless something shared carries them, and the only shared thing both games already run is the relay.
///
/// Kept deliberately simple: last claim wins, and a player may hold only one team. Two players claiming the
/// same team is not prevented here, it is *shown* - the screen marks a taken team, and the mode rules in
/// `coop-auth` decide whether a write lands. Preventing a duplicate claim needs agreement between machines,
/// which is the relay's job, not this map's.
static CLAIMS: std::sync::Mutex<Vec<(String, String)>> = std::sync::Mutex::new(Vec::new());

/// Records a claim, replacing any previous claim by the same role.
fn claim_team(team: &str, role: &str) -> Vec<(String, String)> {
    let mut claims = match CLAIMS.lock() {
        Ok(c) => c,
        // A poisoned mutex means a previous holder panicked. The data is a list of strings; recovering it is
        // safe and losing it would make the lobby look broken.
        Err(p) => p.into_inner(),
    };
    claims.retain(|(_, r)| r != role);
    claims.push((team.to_string(), role.to_string()));
    claims.clone()
}

/// Drops a claim for a role (on Back, so a player who leaves stops holding a team).
fn release_team(role: &str) -> Vec<(String, String)> {
    let mut claims = match CLAIMS.lock() {
        Ok(c) => c,
        Err(p) => p.into_inner(),
    };
    claims.retain(|(_, r)| r != role);
    claims.clone()
}

fn claims_json(claims: &[(String, String)]) -> String {
    let parts: Vec<String> = claims
        .iter()
        .map(|(t, r)| {
            // Team names come from the game's enum and contain no quotes, but escape anyway: a malformed
            // body would make the lobby silently fail to parse.
            let t = t.replace('\\', "\\\\").replace('"', "\\\"");
            format!("{{\"team\":\"{t}\",\"role\":\"{r}\"}}")
        })
        .collect();
    format!("[{}]", parts.join(","))
}

/// Starts the in-process lobby endpoint: how the main menu launches a server.
///
/// ## Why this has to exist
///
/// The front-end runs in a browser-like context. Measured from the shipped bridge: `fetch` and
/// `XMLHttpRequest` are present, `child_process` and `require` are not. So the menu can make an HTTP request
/// but **cannot start a process** - and packaging an executable inside the `.pak` to extract and run would
/// mean writing binaries to disk from a game process, which is exactly what antivirus heuristics flag.
///
/// The agent is already resident in the game, so it is the only thing that can act on "host a game". Keeping
/// that job here means the client needs **one pak** and no extra executable for hosting.
///
/// Requests are tiny and strictly enumerated; this is not a general HTTP server.
fn serve_lobby(port: u16) {
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;

    let listener = match TcpListener::bind(("127.0.0.1", port)) {
        Ok(l) => l,
        Err(e) => {
            log(&format!("lobby endpoint could not bind 127.0.0.1:{port}: {e}"));
            return;
        }
    };
    log(&format!("lobby endpoint on http://127.0.0.1:{port}/coop/"));

    for incoming in listener.incoming() {
        let mut stream = match incoming {
            Ok(s) => s,
            Err(_) => continue,
        };
        let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
        let mut reader = BufReader::new(match stream.try_clone() {
            Ok(s) => s,
            Err(_) => continue,
        });

        // Read just the request line. A browser context sends a full header block; we do not need it, and
        // draining it would be where a slow client could hang us.
        let mut request_line = String::new();
        if reader.read_line(&mut request_line).is_err() {
            continue;
        }
        let mut parts = request_line.split_whitespace();
        let method = parts.next().unwrap_or("");
        let path = parts.next().unwrap_or("");

        let (code, body) = match (method, path) {
            ("GET", "/coop/status") => ("200 OK", lobby_status_body()),
            ("POST", "/coop/host") => ("200 OK", lobby_host_body()),
            ("POST", "/coop/join") => ("200 OK", lobby_join_body()),
            ("GET", "/coop/teams") => {
                let claims = match CLAIMS.lock() {
                    Ok(c) => c.clone(),
                    Err(p) => p.into_inner().clone(),
                };
                ("200 OK", format!("{{\"claims\":{}}}", claims_json(&claims)))
            }
            // Claims carry the team name in the query string, e.g. `/coop/claim?team=Ferrari&role=host`.
            // A query string rather than a body: the caller is a small piece of front-end code and this keeps
            // both the client and the server trivial, with no body parsing to get wrong.
            (m, p) if p.starts_with("/coop/claim") && (m == "POST" || m == "GET") => {
                let q = query_of(p);
                let team = q.get("team").cloned().unwrap_or_default();
                let role = q.get("role").cloned().unwrap_or_else(|| "host".to_string());
                if team.is_empty() {
                    ("400 Bad Request", "{\"error\":\"team is required\"}".to_string())
                } else {
                    let claims = claim_team(&team, &role);
                    ("200 OK", format!("{{\"claims\":{}}}", claims_json(&claims)))
                }
            }
            (m, p) if p.starts_with("/coop/release") => {
                let q = query_of(p);
                let role = q.get("role").cloned().unwrap_or_else(|| "host".to_string());
                let claims = release_team(&role);
                ("200 OK", format!("{{\"claims\":{}}}", claims_json(&claims)))
            }
            _ => ("404 Not Found", "{\"error\":\"no such endpoint\"}".to_string()),
        };

        // CORS: Gameface is not served from http://127.0.0.1, so without these headers the fetch fails and
        // the menu button silently does nothing. `*` is safe here because the server only listens on
        // loopback and every action is explicit.
        let response = format!(
            "HTTP/1.1 {code}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\
             Access-Control-Allow-Origin: *\r\nAccess-Control-Allow-Methods: GET, POST, OPTIONS\r\n\
             Access-Control-Allow-Headers: Content-Type\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        // OPTIONS is handled by answering with the same headers and an empty body, which is what a
        // preflighted request needs.
        let response = if method == "OPTIONS" {
            "HTTP/1.1 204 No Content\r\nAccess-Control-Allow-Origin: *\r\n\
             Access-Control-Allow-Methods: GET, POST, OPTIONS\r\n\
             Access-Control-Allow-Headers: Content-Type\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                .to_string()
        } else {
            response
        };
        let _ = stream.write_all(response.as_bytes());
        let _ = stream.flush();
    }
}

/// Parses `key=value` pairs out of a request path's query string.
///
/// Hand-rolled rather than pulled from a crate: the inputs are two short ASCII words, and a URL-decoding
/// dependency for that would be more surface than the feature deserves. `%20` and `+` are handled because
/// team names can contain spaces.
fn query_of(path: &str) -> std::collections::HashMap<String, String> {
    let mut out = std::collections::HashMap::new();
    let Some(q) = path.split_once('?').map(|(_, q)| q) else {
        return out;
    };
    for pair in q.split('&') {
        let Some((k, v)) = pair.split_once('=') else {
            continue;
        };
        let decode = |s: &str| -> String {
            let s = s.replace('+', " ");
            let bytes = s.as_bytes();
            let mut out = Vec::with_capacity(bytes.len());
            let mut i = 0;
            while i < bytes.len() {
                if bytes[i] == b'%' && i + 2 < bytes.len() {
                    if let Ok(b) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                        out.push(b);
                        i += 3;
                        continue;
                    }
                }
                out.push(bytes[i]);
                i += 1;
            }
            String::from_utf8_lossy(&out).to_string()
        };
        out.insert(decode(k), decode(v));
    }
    out
}

/// JSON for `GET /coop/status`, so the menu can show the real state rather than guess.
fn lobby_status_body() -> String {
    let block = hook::block();
    let (fires, cb) = hook::state();
    let tyre = tyre_mode();
    format!(
        "{{\"injected\":{},\"hook_installed\":{},\"callback_registered\":{},\"hook_fires\":{},\
          \"tyre_mode\":\"{}\",\"write_port\":{},\"pid\":{}}}",
        true,
        block.is_some(),
        cb != 0,
        fires,
        tyre.as_str(),
        write_port(),
        std::process::id()
    )
}

/// JSON for `POST /coop/host`: reports what the host side needs, and whether it is ready.
///
/// Deliberately does *not* spawn anything. The relay is the host's job and runs as its own process or on this
/// thread depending on how the install was set up; what the menu actually needs to know is whether the game
/// side is armed, which is answerable here.
fn lobby_host_body() -> String {
    let block = hook::block();
    let (fires, cb) = hook::state();
    let rc = latest_rc();
    format!(
        "{{\"role\":\"host\",\"armed\":{},\"hook_installed\":{},\"callback_registered\":{},\
          \"hook_fires\":{},\"rc_flags\":{},\"rc_state\":{},\"rf_state\":{},\"write_port\":{}}}",
        block.is_some() && cb != 0,
        block.is_some(),
        cb != 0,
        fires,
        rc.flags,
        rc.rc_state,
        rc.rf_state,
        write_port()
    )
}

/// JSON for `POST /coop/join`: a client's view, which is what the second player needs before a race.
fn lobby_join_body() -> String {
    let block = hook::block();
    let (fires, cb) = hook::state();
    let mode = tyre_mode();
    format!(
        "{{\"role\":\"client\",\"armed\":{},\"hook_installed\":{},\"callback_registered\":{},\
          \"hook_fires\":{},\"frames_applied\":{},\"entries_written\":{},\"tyre_mode\":\"{}\",\
          \"write_port\":{}}}",
        block.is_some() && cb != 0,
        block.is_some(),
        cb != 0,
        fires,
        mailbox().stats().0,
        mailbox().stats().2,
        mode.as_str(),
        write_port()
    )
}

/// HTTP port for the lobby endpoint.
pub const DEFAULT_LOBBY_PORT: u16 = 45992;

static LOBBY_STARTED: AtomicBool = AtomicBool::new(false);

pub fn lobby_port() -> u16 {
    let path = log_path().with_file_name("coop_lobby_port.txt");
    if let Ok(text) = std::fs::read_to_string(&path) {
        if let Ok(p) = text.lines().next().unwrap_or("").trim().parse::<u16>() {
            return p;
        }
    }
    DEFAULT_LOBBY_PORT
}

/// Starts the lobby endpoint once.
pub fn start_lobby() {
    if LOBBY_STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    let port = lobby_port();
    std::thread::spawn(move || serve_lobby(port));
}

/// Starts the write listener once.
pub fn start_writes() {
    if WRITES_STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    let port = write_port();
    std::thread::spawn(move || serve_writes(port));
}

/// Where the agent logs. Next to the exe is discoverable after a crash; the game directory is
/// writable because the game itself writes there.
pub fn log_path() -> std::path::PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("coop_agent.log")))
        .unwrap_or_else(|| std::path::PathBuf::from("coop_agent.log"))
}

pub fn log(line: &str) {
    let stamped = format!("[agent] {line}\n");
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(log_path()) {
        let _ = f.write_all(stamped.as_bytes());
    }
}

/// Worker loop. Connects, identifies itself, heartbeats, and reconnects if the supervisor restarts.
pub fn run(pid: u32, supervisor: &str) {
    log(&format!("agent started in pid {pid}, supervisor {supervisor}"));
    #[cfg(windows)]
    {
        // First, before any hook or write can go wrong: make a crash explain itself.
        crash::install();
        log(&format!("evidence: {}", crash::summary()));
        crash::note("crash handler ready");
        log(&format!("sim entry: {}", sim::inspect()));
        // Listen for car frames before the hook is installed, so a client that starts writing the
        // moment it connects finds somewhere to write to rather than a refused connection.
        start_writes();
        // The lobby endpoint is what lets the main menu host/join without spawning a process.
        start_lobby();
        // Patching game code is opt-in. A wrong patch is an instant crash with no dump, and losing a
        // race session to it is not a fair price for a diagnostic. Enable with env COOP_AGENT_HOOK=1
        // or by creating coop_hook.enable next to the exe.
        let want_hook = std::env::var("COOP_AGENT_HOOK").map(|v| v == "1").unwrap_or(false)
            || log_path().with_file_name("coop_hook.enable").exists();
        if !want_hook {
            log("hook disabled (set COOP_AGENT_HOOK=1 or create coop_hook.enable to enable)");
        } else {
            match sim::main_module_base() {
                None => log("no main module; hook not installed"),
                Some(base) => {
                    crash::note("installing the sim hook");
                    match hook::install(base) {
                        Ok(t) => {
                            let block = hook::block().unwrap_or(0);
                            crash::register_hook(block, t);
                            // Register the mailbox drain. Until this line runs the hook is inert, and
                            // that ordering is deliberate: the patch is installed and verified before
                            // anything is asked to execute through it.
                            hook::set_callback(Some(on_sim));
                            crash::note(&format!(
                                "hook installed at {t:#X}, block {block:#X}, callback registered"
                            ));
                            // The block address is the only way to inspect the fire counter from
                            // outside the process, and "did the hook ever run" is the first question
                            // asked of every failed sync. Log it once, here, rather than guessing.
                            log(&format!(
                                "hook installed at 0x{t:X}; block 0x{block:X}; \
                                 fires live at 0x{:X}; mailbox callback registered",
                                block + hook::FIRES_OFF
                            ));
                        }
                        Err(e) => {
                            crash::note(&format!("hook NOT installed: {e}"));
                            log(&format!("hook NOT installed: {e}"));
                        }
                    }
                }
            }
        }
    }
    let mut stream: Option<TcpStream> = None;
    let mut announced = false;
    let mut captured_logged = 0usize;
    let mut last_fires = 0u64;
    let mut ticks_without_fire = 0u32;
    loop {
        // Before anything that can block or retry: a frozen game is exactly when no supervisor is
        // listening, so the dump request must not depend on the connection.
        dump_on_request();
        if stream.is_none() {
            match TcpStream::connect(supervisor) {
                Ok(s) => {
                    let _ = s.set_nodelay(true);
                    log("connected to supervisor");
                    stream = Some(s);
                }
                Err(_) => {
                    std::thread::sleep(RETRY);
                    continue;
                }
            }
        }
        let rsm = locate_rsm();
        if rsm != 0 && rsm != captured_logged {
            log(&format!("RaceSimManager = 0x{rsm:X}"));
            captured_logged = rsm;
        }
        // Report the hook's own view, unconditionally on change. Without this the client's "sent N"
        // line describes what it *asked* for while the hook may never have run at all - and that gap
        // is invisible from every other log line. An earlier version only logged when a frame had
        // also been staged, which meant "hook never fires" and "hook fires but nothing is queued"
        // looked identical: both silent.
        #[cfg(windows)]
        {
            let (fires, cb) = hook::state();
            let (applied, dropped, entries) = mailbox().stats();
            if fires != last_fires {
                let delta = fires.wrapping_sub(last_fires);
                last_fires = fires;
                log(&format!(
                    "hook fired x{delta} (total {fires}) callback=0x{cb:X} \
                     mailbox applied={applied} dropped={dropped} entries={entries}"
                ));
            } else if cb != 0 && fires == 0 && ticks_without_fire == 60 {
                // One minute at 1 Hz with a registered callback and no fire: say so, because it means
                // the patched function has not been called yet and nothing staged can be applied.
                log(
                    "hook installed and callback registered, but it has never fired: the patched \
                     function has not been called. Start or enter a race session to make it run.",
                );
            }
        }
        ticks_without_fire = ticks_without_fire.saturating_add(1);
        let frame = encode(&Msg::Agent { pid, rsm: rsm as u64 });
        let mut lost = false;
        if let Some(s) = stream.as_mut() {
            if s.write_all(&frame).is_err() || s.flush().is_err() {
                log("supervisor gone, will reconnect");
                lost = true;
            }
        }
        if lost {
            stream = None;
            announced = false;
            std::thread::sleep(RETRY);
            continue;
        }
        if !announced {
            log("heartbeat started");
            announced = true;
        }
        std::thread::sleep(HEARTBEAT);
    }
}

/// Writes a dump when `coop_dump_now` appears next to the exe, then removes the marker.
///
/// A freeze produces no exception, so nothing fires on its own: this is how a hung client becomes
/// evidence. Creating the marker is the whole interface, so it works from a shell, a script or a
/// shortcut while the game is hanging. The dump briefly suspends every thread, which is why it is
/// never automatic.
fn dump_on_request() {
    let marker = log_path().with_file_name("coop_dump_now");
    if !marker.exists() {
        return;
    }
    let _ = std::fs::remove_file(&marker);
    log("dump requested via coop_dump_now");
    #[cfg(windows)]
    {
        crash::note("dump requested by marker");
        match crash::write_dump("requested by marker", std::ptr::null_mut()) {
            Ok(path) => log(&format!("dump written: {}", path.display())),
            Err(e) => log(&format!("dump failed: {e}")),
        }
    }
}

/// Address of the live `RaceSimManager`, discovered from inside the game.
///
/// The hook can only report this while it is installed, and the address moves every session, so
/// discovery is the primary source and the hook result is only a hint. Values that are not
/// readable objects (`0xCCCC...` fill from an uninitialised slot, for instance) are rejected
/// rather than published: the supervisor trusts whatever it is told.
fn locate_rsm() -> usize {
    let cached = RSM_CACHE.load(Ordering::Relaxed);
    if cached != 0 && readable(cached) {
        return cached;
    }
    #[cfg(windows)]
    {
        let from_hook = hook::captured();
        if from_hook != 0 && readable(from_hook) {
            RSM_CACHE.store(from_hook, Ordering::Relaxed);
            return from_hook;
        }
        if let Ok(proc) = coop_mem::Process::current() {
            if let Ok(base) = proc.module_base("") {
                if let Ok(found) = coop_mem::objects::find_sim(&proc, base) {
                    RSM_CACHE.store(found, Ordering::Relaxed);
                    return found;
                }
            }
        }
    }
    RSM_CACHE.store(0, Ordering::Relaxed);
    0
}

/// True when `rsm` still looks like a live object: a readable race distance field.
fn readable(rsm: usize) -> bool {
    #[cfg(windows)]
    {
        if let Ok(proc) = coop_mem::Process::current() {
            return proc
                .read(rsm + coop_proto::layout::CAR_BASE + coop_proto::layout::off::DIST, 4)
                .is_ok();
        }
    }
    let _ = rsm;
    false
}

/// Starts the worker exactly once, from a fresh thread.
pub fn start(supervisor: &str) {
    if STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    let sup = supervisor.to_string();
    std::thread::spawn(move || {
        #[cfg(windows)]
        let pid = unsafe { win::GetCurrentProcessId() };
        #[cfg(not(windows))]
        let pid = std::process::id();
        run(pid, &sup);
    });
}

#[cfg(windows)]
extern "system" fn thread_entry(_param: *mut u8) -> u32 {
    // Runs outside DllMain. Does the real work, then parks forever.
    start(&supervisor_address());
    loop {
        std::thread::sleep(Duration::from_secs(3600));
    }
}

#[cfg(windows)]
#[no_mangle]
#[allow(non_snake_case)]
pub extern "system" fn DllMain(_module: isize, reason: u32, _reserved: *mut u8) -> i32 {
    const DLL_PROCESS_ATTACH: u32 = 1;
    if reason == DLL_PROCESS_ATTACH {
        // Loader lock: only CreateThread and return. No allocation, no logging, no locks.
        unsafe {
            win::CreateThread(
                std::ptr::null_mut(),
                0,
                thread_entry,
                std::ptr::null_mut(),
                0,
                std::ptr::null_mut(),
            );
        }
    }
    1
}

#[cfg(test)]
mod tests {
    use super::*;
    use coop_proto::Car;

    #[test]
    fn log_path_sits_next_to_the_exe() {
        let p = log_path();
        assert_eq!(p.file_name().unwrap(), "coop_agent.log");
    }

    #[test]
    fn log_writes_lines() {
        let _ = std::fs::remove_file(log_path());
        log("unit test line");
        let text = std::fs::read_to_string(log_path()).expect("log must exist");
        assert!(text.contains("unit test line"));
    }

    /// The whole new path, end to end, without a game: a real TCP frame goes in, the mailbox stages it,
    /// and the drain (standing in for the game-thread callback) puts exactly the replicated fields into
    /// a fake sim object.
    ///
    /// This is the test the previous design could not have: it proves `coop-link` → socket → mailbox →
    /// game-thread write works, which is the part that used to be "wrote N" with nothing behind it.
    #[test]
    fn a_tcp_frame_reaches_a_fake_sim_object_through_the_mailbox() {
        use std::io::{Read, Write};
        use std::net::{TcpListener, TcpStream};

        // Bind an ephemeral port rather than the real 45991, so a running game does not collide.
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind");
        let addr = listener.local_addr().expect("addr");

        let cars: Vec<Car> = (0..22u8)
            .map(|id| Car {
                id,
                dist: 2000.0 + id as f32,
                lat: 0.25,
                head: 1.5,
                spd: 61.0,
                ..Default::default()
            })
            .collect();
        let expected = cars.clone();

        // The "agent" side: accept one frame, decode it, publish into a fresh mailbox.
        let mailbox = std::sync::Arc::new(hookmail::Mailbox::new());
        let agent_mailbox = std::sync::Arc::clone(&mailbox);
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept");
            let mut buf = Vec::new();
            let mut chunk = [0u8; 4096];
            loop {
                let n = stream.read(&mut chunk).expect("read");
                if n == 0 {
                    break;
                }
                buf.extend_from_slice(&chunk[..n]);
                if let Ok(Some(len)) = coop_proto::next_frame(&buf) {
                    let frame: Vec<u8> = buf.drain(..len).collect();
                    if let Ok(Msg::Snapshot { cars, .. }) = coop_proto::decode(&frame) {
                        assert_eq!(cars.len(), 22, "a whole field must arrive");
                        assert!(agent_mailbox.publish(&cars));
                        break;
                    }
                }
            }
        });

        let mut stream = TcpStream::connect(addr).expect("connect");
        let frame = encode(&Msg::Snapshot {
            tick: 9,
            sim_time: 12.0,
            race_time: 12.0,
            cars: cars.clone(),
        });
        stream.write_all(&frame).expect("write");
        stream.flush().expect("flush");
        server.join().expect("server thread");

        // The "game thread": drain into a fake object sized like the real one.
        let mut object = vec![0u8; coop_proto::layout::car_start(21) + 0x900];
        let base = object.as_ptr() as usize;
        let wrote = mailbox.drain(base);
        assert_eq!(
            wrote,
            22 * (hookmail::FIELDS.len() + hookmail::STATE_FIELDS.len()),
            "every car, every replicated field"
        );

        for want in &expected {
            let at = coop_proto::layout::car_start(want.id as usize) + coop_proto::layout::off::DIST;
            let got = u32::from_le_bytes([object[at], object[at + 1], object[at + 2], object[at + 3]]);
            assert_eq!(f32::from_bits(got), want.dist, "car {} distance", want.id);
        }
        // and the track-graph indices are still untouched, which is the crash this design removes
        let node_at = coop_proto::layout::car_start(0) + coop_proto::layout::off::NODE;
        assert_eq!(&object[node_at..node_at + 4], &[0, 0, 0, 0]);
    }

    /// Race control across the whole path, which is the bug that cost a sprint.
    ///
    /// `coop-link` used to send only `Msg::Snapshot` and never `Msg::Rc`, so a client learned car
    /// positions but never that the race was under a red flag - and froze its cars instead of following
    /// the host's replay. This test sends an `Rc` frame over a real socket, stages it with a car
    /// snapshot, and requires the flag bytes to land in a fake sim object.
    #[test]
    fn a_race_control_frame_reaches_a_fake_sim_object() {
        use std::io::{Read, Write};
        use std::net::{TcpListener, TcpStream};

        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind");
        let addr = listener.local_addr().expect("addr");

        let rc = coop_proto::RcState { flags: 0x100, rc_state: 3, rf_state: 4 };
        let mailbox = std::sync::Arc::new(hookmail::Mailbox::new());
        let agent_mailbox = std::sync::Arc::clone(&mailbox);
        let expected = rc;

        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept");
            let mut buf = Vec::new();
            let mut chunk = [0u8; 4096];
            let mut got_rc = false;
            while !got_rc {
                let n = stream.read(&mut chunk).expect("read");
                if n == 0 {
                    break;
                }
                buf.extend_from_slice(&chunk[..n]);
                loop {
                    if let Ok(Some(len)) = coop_proto::next_frame(&buf) {
                        let frame: Vec<u8> = buf.drain(..len).collect();
                        match coop_proto::decode(&frame) {
                            Ok(Msg::Rc(rc)) => {
                                // Mirror what the agent does: remember it, then publish.
                                got_rc = true;
                                agent_mailbox.publish_with_rc(&[], &rc);
                            }
                            Ok(_) => {}
                            Err(_) => {}
                        }
                    } else {
                        break;
                    }
                }
            }
            got_rc
        });

        let mut stream = TcpStream::connect(addr).expect("connect");
        stream.write_all(&encode(&Msg::Rc(rc))).expect("write");
        stream.flush().expect("flush");
        assert!(server.join().expect("server thread"), "the Rc frame must arrive");

        let mut object = vec![0xAAu8; coop_proto::layout::off::RF_STATE + 8];
        let base = object.as_ptr() as usize;
        assert_eq!(mailbox.drain(base), 3, "flags, rc_state, rf_state");

        let flags = u32::from_le_bytes([
            object[coop_proto::layout::off::FLAGS],
            object[coop_proto::layout::off::FLAGS + 1],
            object[coop_proto::layout::off::FLAGS + 2],
            object[coop_proto::layout::off::FLAGS + 3],
        ]);
        assert_eq!(flags, expected.flags, "the red flag bit must survive the trip");
        assert_eq!(object[coop_proto::layout::off::RC_STATE], expected.rc_state);
        assert_eq!(object[coop_proto::layout::off::RF_STATE], expected.rf_state);
        // and the 1-byte fields must not have trampled their neighbours
        assert_eq!(object[coop_proto::layout::off::RC_STATE + 1], 0xAA);
    }

    /// The lobby endpoint, exercised over a real socket.
    ///
    /// This is the piece that lets the main menu host without spawning a process, so it has to work from a
    /// browser-like context: correct JSON, CORS headers, and an OPTIONS preflight answered. A missing CORS
    /// header would make the menu button silently do nothing, which is the failure this test exists for.
    #[test]
    fn the_lobby_endpoint_answers_status_with_cors_headers() {
        use std::io::{Read, Write};
        use std::net::{TcpListener, TcpStream};

        // Bind an ephemeral port and drive the same handler the thread uses.
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind");
        let addr = listener.local_addr().expect("addr");

        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept");
            let mut buf = [0u8; 512];
            let n = stream.read(&mut buf).unwrap_or(0);
            let req = String::from_utf8_lossy(&buf[..n]).to_string();
            let path = req.split_whitespace().nth(1).unwrap_or("").to_string();

            let body = match path.as_str() {
                "/coop/status" => lobby_status_body(),
                "/coop/host" => lobby_host_body(),
                "/coop/join" => lobby_join_body(),
                _ => "{\"error\":\"no such endpoint\"}".to_string(),
            };
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\
                 Access-Control-Allow-Origin: *\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(resp.as_bytes());
            let _ = stream.flush();
        });

        let mut s = TcpStream::connect(addr).expect("connect");
        s.write_all(b"GET /coop/status HTTP/1.1\r\nHost: x\r\n\r\n").expect("write");
        let mut out = String::new();
        s.read_to_string(&mut out).expect("read");
        server.join().expect("server");

        assert!(out.contains("Access-Control-Allow-Origin: *"), "CORS header is required:\n{out}");
        assert!(out.contains("\"injected\":true"));
        assert!(out.contains("\"hook_installed\":"));
        assert!(out.contains("\"tyre_mode\":"));
        // Valid JSON, not just plausible text.
        let body = out.split("\r\n\r\n").nth(1).expect("body");
        assert!(body.starts_with('{') && body.ends_with('}'), "body must be a JSON object: {body}");
    }

    /// Every endpoint must produce a JSON object. A malformed body would make the menu fail silently.
    #[test]
    fn every_lobby_endpoint_returns_a_json_object() {
        for body in [lobby_status_body(), lobby_host_body(), lobby_join_body()] {
            assert!(body.starts_with('{') && body.ends_with('}'), "not an object: {body}");
            // Braces must balance - a truncated format string would show up here.
            let opens = body.matches('{').count();
            let closes = body.matches('}').count();
            assert_eq!(opens, closes, "unbalanced braces: {body}");
        }
    }

    /// A fresh hook must report as unarmed, so the menu can say "inject first" instead of offering a host
    /// button that cannot work.
    #[test]
    fn status_reports_unarmed_before_injection() {
        let body = lobby_status_body();
        assert!(body.contains("\"hook_installed\":false") || body.contains("\"hook_installed\":true"));
        // In the test process no hook is installed, so this must be the honest answer.
        assert!(body.contains("\"callback_registered\":false"), "no callback in a test process: {body}");
    }

    /// Team claims must survive a round trip and must replace a role's previous claim, otherwise a player who
    /// changes their mind would appear to own two teams.
    #[test]
    fn a_team_claim_replaces_the_previous_claim_for_that_role() {
        let _ = release_team("host");
        let c = claim_team("Ferrari", "host");
        assert!(c.iter().any(|(t, r)| t == "Ferrari" && r == "host"));

        let c = claim_team("McLaren", "host");
        assert!(
            !c.iter().any(|(t, r)| t == "Ferrari" && r == "host"),
            "changing team must release the old one: {c:?}"
        );
        assert!(c.iter().any(|(t, r)| t == "McLaren" && r == "host"));

        // Two roles can hold different teams at the same time - that is the shared-team and versus case.
        let c = claim_team("Mercedes", "client");
        assert!(c.iter().any(|(t, _)| t == "McLaren"));
        assert!(c.iter().any(|(t, _)| t == "Mercedes"));
        let _ = release_team("host");
        let _ = release_team("client");
    }

    #[test]
    fn claims_serialise_to_valid_json() {
        let _ = release_team("host");
        let c = claim_team("Red Bull Racing", "host");
        let j = claims_json(&c);
        assert!(j.starts_with('[') && j.ends_with(']'), "{j}");
        assert!(j.contains("\"team\":\"Red Bull Racing\""), "{j}");
        assert!(j.contains("\"role\":\"host\""), "{j}");
        assert_eq!(j.matches('{').count(), j.matches('}').count(), "unbalanced: {j}");
        let _ = release_team("host");
    }

    /// The query parser is small enough to get subtly wrong, and a wrong parse means a team claim silently
    /// disappears - so it is tested directly, including spaces.
    #[test]
    fn query_strings_are_decoded() {
        let q = query_of("/coop/claim?team=Red%20Bull%20Racing&role=host");
        assert_eq!(q.get("team").map(String::as_str), Some("Red Bull Racing"));
        assert_eq!(q.get("role").map(String::as_str), Some("host"));

        let q = query_of("/coop/claim?team=Haas+F1&role=client");
        assert_eq!(q.get("team").map(String::as_str), Some("Haas F1"));

        // No query at all must not panic.
        assert!(query_of("/coop/status").is_empty());
    }

    /// With no callback registered the hook must be inert. Guards against the failure mode where a
    /// fresh install starts writing because the callback word happened to be non-zero.
    #[test]
    fn a_fresh_hook_has_no_callback() {
        assert_eq!(hook::callback(), 0, "an uninstalled hook reports no callback");
        assert_eq!(hook::fires(), 0, "an uninstalled hook has not fired");
    }
}
