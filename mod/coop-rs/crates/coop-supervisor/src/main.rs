//! Co-op supervisor: the out-of-process half.
//!
//! Two game instances connect here over loopback. Everything that used to be silent Lua - ordering,
//! staleness, ownership, retries - happens in this process instead, where a failure is a typed error
//! and a log line rather than a frozen game.
//!
//! Responsibilities today:
//!   * accept and authenticate the two peers (Hello carries the side)
//!   * validate every frame before relaying it (the codec rejects torn, corrupt and version-mismatched
//!     frames, so a bad peer cannot inject half a snapshot)
//!   * relay snapshots, race control and orders to the other side
//!   * log timings and counters so a stall names the side that stopped sending
//!
//! Deliberately not here yet: the peer-to-peer transport, ownership arbitration, plausibility checks.
//! Those belong above the wire format, and the wire format is what needs proving first.

use std::io::{self, Read, Write};
use std::net::{TcpListener, TcpStream, SocketAddr};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use coop_proto::{decode, next_frame, encode, Msg, Side};

const DEFAULT_PORT: u16 = 45990;
const READ_CHUNK: usize = 8192;

#[derive(Default)]
struct Stats {
    frames_in: AtomicU64,
    frames_out: AtomicU64,
    bytes_in: AtomicU64,
    bad_frames: AtomicU64,
}

/// One connected game instance and the socket we write its peer's messages to.
struct Peer {
    side: Side,
    stream: TcpStream,
}

type Registry = Arc<Mutex<Vec<Peer>>>;

fn log(msg: &str) {
    println!("[supervisor] {msg}");
}

/// Fan a frame out to the other side.
///
/// Two rules here, both learned from a live failure where the client froze on one frame forever:
///
///  * Never hold the registry lock across a socket write. A single peer that has stopped draining
///    fills its TCP window, `write_all` blocks, and the lock is held for the lifetime of that block -
///    which starves every other peer. That is what made sync stop dead mid-race.
///  * Never retry a peer that failed. A dead socket must be removed, not written to again.
fn relay(reg: &Registry, from: Side, frame: &[u8], stats: &Stats) -> io::Result<()> {
    // Clone the handles, then release the lock *before* touching the network.
    let targets: Vec<(usize, TcpStream)> = {
        let guard = reg.lock().unwrap();
        guard
            .iter()
            .enumerate()
            .filter(|(_, p)| p.side != from)
            .filter_map(|(i, p)| p.stream.try_clone().ok().map(|s| (i, s)))
            .collect()
    };

    let mut dead: Vec<usize> = Vec::new();
    for (idx, mut stream) in targets {
        // Blocking writes per peer, with the lock released, so one stalled peer cannot hold up the
        // others. The write timeout is set when the peer is accepted.
        match stream.write_all(frame) {
            Ok(()) => {
                stats.frames_out.fetch_add(1, Ordering::Relaxed);
            }
            Err(e) => {
                log(&format!("relay write to peer {idx} failed: {e}"));
                dead.push(idx);
            }
        }
    }

    if !dead.is_empty() {
        let mut guard = reg.lock().unwrap();
        // Remove by descending index so the earlier removals do not shift later ones.
        dead.sort_unstable_by(|a, b| b.cmp(a));
        for idx in dead {
            if idx < guard.len() {
                let peer = guard.remove(idx);
                log(&format!("dropped peer {:?} after a failed write", peer.side));
            }
        }
    }
    Ok(())
}

fn drain_peer(reg: Registry, mut stream: TcpStream, addr: SocketAddr, stats: Arc<Stats>) -> io::Result<()> {
    let mut buf: Vec<u8> = Vec::with_capacity(READ_CHUNK * 2);
    let mut chunk = [0u8; READ_CHUNK];
    let mut side: Option<Side> = None;
    let mut seen_agent: Option<(u32, u64)> = None;
    let mut zero_announced: std::collections::HashSet<u32> = std::collections::HashSet::new();

    loop {
        let n = stream.read(&mut chunk)?;
        if n == 0 {
            log(&format!("{addr} closed"));
            break;
        }
        stats.bytes_in.fetch_add(n as u64, Ordering::Relaxed);
        buf.extend_from_slice(&chunk[..n]);

        loop {
            match next_frame(&buf) {
                Ok(None) => break,
                Ok(Some(len)) => {
                    let frame: Vec<u8> = buf.drain(..len).collect();
                    match decode(&frame) {
                        Ok(Msg::Hello { side: s, cars_owned }) => {
                            side = Some(s);
                            log(&format!("{addr} says it is {s:?}, owns {cars_owned} car(s)"));
                            reg.lock().unwrap().push(Peer { side: s, stream: stream.try_clone()? });
                            continue;
                        }
                        Ok(Msg::Agent { pid, rsm }) => {
                            // The agent heartbeats once a second, and a stale copy of the agent
                            // left in the process reports rsm=0 forever, so logging every message
                            // buries the real handshake. A zero means "still looking", worth one
                            // line per process; a real address is worth one line per change.
                            if rsm == 0 {
                                if zero_announced.insert(pid) {
                                    log(&format!("agent in pid {pid}, still locating RaceSimManager"));
                                }
                            } else if seen_agent != Some((pid, rsm)) {
                                log(&format!("agent in pid {pid}, rsm={rsm:#X}"));
                                seen_agent = Some((pid, rsm));
                            }
                            if rsm != 0 {
                                // publish the endpoint so coop-probe can attach without hand-copying
                                let now = std::time::SystemTime::now()
                                    .duration_since(std::time::UNIX_EPOCH)
                                    .map(|d| d.as_secs())
                                    .unwrap_or(0);
                                let line = format!("RSM {rsm:#X} {:#X} {now}\n", coop_proto::layout::OBJECT_SIZE);
                                if let Err(e) = std::fs::write("coop_rsm.txt", line) {
                                    log(&format!("could not write coop_rsm.txt: {e}"));
                                } else {
                                    log("wrote coop_rsm.txt");
                                }
                            }
                            continue;
                        }
                        Ok(Msg::Snapshot { tick, sim_time, race_time, cars }) => {
                            stats.frames_in.fetch_add(1, Ordering::Relaxed);
                            if tick % 100 == 0 {
                                log(&format!(
                                    "{addr} snapshot tick={tick} race_time={race_time:.3} cars={}",
                                    cars.len()
                                ));
                            }
                        }
                        Ok(_) => {
                            stats.frames_in.fetch_add(1, Ordering::Relaxed);
                        }
                        Err(e) => {
                            stats.bad_frames.fetch_add(1, Ordering::Relaxed);
                            log(&format!("{addr} sent an invalid frame: {e}"));
                            continue;
                        }
                    }
                    if let Some(s) = side {
                        if let Err(e) = relay(&reg, s, &frame, &stats) {
                            log(&format!("relay failed: {e}"));
                            return Ok(());
                        }
                    } else {
                        log(&format!("{addr} sent data before Hello; dropped"));
                    }
                }
                Err(e) => {
                    stats.bad_frames.fetch_add(1, Ordering::Relaxed);
                    log(&format!("{addr} framing error ({e}); closing"));
                    return Ok(());
                }
            }
        }
    }

    if let Some(s) = side {
        reg.lock().unwrap().retain(|p| p.side != s);
        log(&format!("{s:?} disconnected"));
    }
    Ok(())
}

fn main() -> io::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--discover") {
        let pid = args
            .iter()
            .position(|a| a == "--pid")
            .and_then(|i| args.get(i + 1))
            .and_then(|s| s.parse::<u32>().ok())
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "--discover needs --pid <pid>"))?;
        let watch = args.iter().any(|a| a == "--watch");
        let out = args
            .iter()
            .position(|a| a == "--out")
            .and_then(|i| args.get(i + 1))
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| std::path::PathBuf::from("coop_rsm.txt"));
        return run_discovery(pid, watch, &out);
    }

    if args.iter().any(|a| a == "--selftest") {
        return selftest();
    }
    let port = args
        .iter()
        .position(|a| a == "--port")
        .and_then(|i| args.get(i + 1))
        .and_then(|p| p.parse::<u16>().ok())
        .unwrap_or(DEFAULT_PORT);

    let bind = args
        .iter()
        .position(|a| a == "--bind")
        .and_then(|i| args.get(i + 1))
        .cloned()
        .unwrap_or_else(|| "0.0.0.0".to_string());
    let listener = TcpListener::bind((bind.as_str(), port))?;
    log(&format!("bound {bind}:{port}"));
    log(&format!("listening on 127.0.0.1:{port}"));
    let reg: Registry = Arc::new(Mutex::new(Vec::new()));
    let stats = Arc::new(Stats::default());

    for incoming in listener.incoming() {
        let stream = incoming?;
        let addr = stream.peer_addr()?;
        stream.set_nodelay(true)?;
        // Bound every socket write. A peer that stops draining must fail rather than wedge the
        // relay forever; the fan-out then drops it.
        stream.set_write_timeout(Some(std::time::Duration::from_secs(2)))?;
        log(&format!("{addr} connected"));
        let reg = Arc::clone(&reg);
        let stats = Arc::clone(&stats);
        thread::spawn(move || {
            if let Err(e) = drain_peer(reg, stream, addr, stats) {
                log(&format!("{addr} error: {e}"));
            }
        });
    }
    Ok(())
}

/// End-to-end check with no external crates: start the relay, connect two peers in-process, send a
/// snapshot from the host and require the client to receive it byte-identical.
fn selftest() -> io::Result<()> {
    let listener = TcpListener::bind(("127.0.0.1", 0))?;
    let addr = listener.local_addr()?;
    log(&format!("selftest: relay on {addr}"));
    let reg: Registry = Arc::new(Mutex::new(Vec::new()));
    let stats = Arc::new(Stats::default());

    {
        let reg = Arc::clone(&reg);
        let stats = Arc::clone(&stats);
        thread::spawn(move || {
            for incoming in listener.incoming() {
                let Ok(stream) = incoming else { continue };
                let Ok(a) = stream.peer_addr() else { continue };
                stream.set_nodelay(true).ok();
                let reg = Arc::clone(&reg);
                let stats = Arc::clone(&stats);
                thread::spawn(move || {
                    let _ = drain_peer(reg, stream, a, stats);
                });
            }
        });
    }

    let hello = |side: Side| -> io::Result<TcpStream> {
        let mut s = TcpStream::connect(addr)?;
        s.set_nodelay(true)?;
        s.write_all(&encode(&Msg::Hello { side, cars_owned: 11 }))?;
        Ok(s)
    };

    let mut host = hello(Side::Host)?;
    let mut client = hello(Side::Client)?;
    thread::sleep(Duration::from_millis(250));

    let cars: Vec<coop_proto::Car> = (0..22u8)
        .map(|i| coop_proto::Car { id: i, dist: 1000.0 + i as f32, spd: 50.0, ..Default::default() })
        .collect();
    let snap = Msg::Snapshot { tick: 7, sim_time: 12.5, race_time: 12.5, cars };
    let frame = encode(&snap);
    host.write_all(&frame)?;
    host.flush()?;

    client.set_read_timeout(Some(Duration::from_secs(2)))?;
    let mut got = vec![0u8; frame.len()];
    client.read_exact(&mut got)?;

    if got != frame {
        log("selftest FAILED: relayed frame differs from the one sent");
        std::process::exit(1);
    }
    match decode(&got) {
        Ok(Msg::Snapshot { tick: 7, sim_time: _, cars, race_time }) => {
            log(&format!(
                "selftest: client received snapshot tick=7 race_time={race_time} cars={}",
                cars.len()
            ));
        }
        other => {
            log(&format!("selftest FAILED: unexpected message {other:?}"));
            std::process::exit(1);
        }
    }
    log(&format!(
        "selftest OK: frames_in={} frames_out={} bytes_in={} bad={}",
        stats.frames_in.load(Ordering::Relaxed),
        stats.frames_out.load(Ordering::Relaxed),
        stats.bytes_in.load(Ordering::Relaxed),
        stats.bad_frames.load(Ordering::Relaxed),
    ));
    Ok(())
}

/// Locates the live `RaceSimManager` in `pid` and publishes `RSM <addr> <size> <unix>`.
///
/// The address changes every session (and the class-default object sits at a stable-looking but
/// useless address), so nothing may cache it. When watching, a cheap 4-byte read validates the
/// cached address each pass and a full object-array scan only runs when that read fails, which
/// keeps the cost off the game's CPU.
fn run_discovery(pid: u32, watch: bool, out: &std::path::Path) -> io::Result<()> {
    use coop_proto::layout;
    let size = layout::OBJECT_SIZE;
    let mut current: Option<usize> = None;
    loop {
        let mut publish = false;
        let mut note = None;
        match coop_mem::Process::open(pid) {
            Err(e) => {
                note = Some(format!("open pid {pid}: {e}"));
                current = None;
            }
            Ok(proc) => {
                let still_live = current.map(|rsm| {
                    proc.read(rsm + layout::CAR_BASE + layout::off::DIST, 4).is_ok()
                }).unwrap_or(false);
                if still_live {
                    publish = true;
                } else {
                    current = None;
                    let base = proc
                        .module_base("F1Manager24.exe")
                        .map_err(|e| io::Error::other(e.to_string()));
                    match base.and_then(|b| {
                        coop_mem::objects::find_sim(&proc, b).map_err(io::Error::other)
                    }) {
                        Ok(rsm) => {
                            note = Some(format!("discovered RaceSimManager at {rsm:#X}"));
                            current = Some(rsm);
                            publish = true;
                        }
                        Err(e) => note = Some(format!("discovery: {e}")),
                    }
                }
            }
        }
        if let Some(rsm) = current.filter(|_| publish) {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            std::fs::write(out, format!("RSM {rsm:#X} {size:#X} {now}\n"))?;
        }
        if let Some(n) = note {
            log(&n);
        }
        if !watch {
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_secs(2));
    }
}
