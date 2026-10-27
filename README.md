<div align="center">

# F1 Manager 2024 — Co-op

**Two players, one career.** A co-op mod for F1 Manager 2024, built from the outside because the game
has no multiplayer at all.

![status](https://img.shields.io/badge/status-prototype-orange)
![version](https://img.shields.io/badge/game-1.11.0.39891%20(Steam%202591280)-blue)
![engine](https://img.shields.io/badge/engine-UE%205.1-lightgrey)
![tests](https://img.shields.io/badge/tests-99%20passing-brightgreen)
![sync](https://img.shields.io/badge/race%20sync-not%20yet-red)

[What](#what) · [Goal](#goal) · [How](#how) · [Progress](#progress) · [Build](#build) · [Plan](#plan) · [Problems](#problems-worth-reading)

</div>

> [!IMPORTANT]
> **Not playable yet.** Race synchronisation does not hold. Everything below is honest about that,
> including the parts that work and the parts that only look like they work. If you are here to play,
> come back later; if you are here to help, [Problems](#problems-worth-reading) is the fastest way in.

---

## What

F1 Manager 2024 is a single-player Formula 1 management sim: contracts, car development, facilities,
race strategy, and a race simulation of 22 cars. Two people cannot play it together. This project makes
that possible.

There is no co-op mod to build on. There is no netcode in the game to lean on — no multiplayer, no
lobbies, no session layer. F1 Manager 2024 is a single-player game with a heavily simulated match, so
everything has to be built from the outside: read the game's state, decide what one machine owns, and
make the other machine agree.

**Scope is the whole game, not just the race.** A co-op career that only syncs races is not a co-op
career. Contracts, transfers, car development, research, facilities, finances and the calendar all
have to work too.

### Three modes

| Mode | Who owns what |
|---|---|
| **Shared team** | One garage, two drivers. Each player runs their own car. Facilities, research and the budget come from player 1. |
| **Versus** | A team each, racing each other. Shared calendar and results, separate garages. |
| **Co-op career** | Two teams, one career. Shared progression and calendar, separate garages. |

The database decides how "my side of the garage" is defined, and it is already clear from the save
schema: `Save_CarConfig` is keyed by `TeamID` **and** `LoadoutID`, `Save_DriverCommands` and
`Save_PlayerStrategyStints` by `LoadoutID`, and `Buildings_HQ` by `TeamID`. So a player's cars are a
`LoadoutID` and a team's facilities are a `TeamID`. That mapping is what the authorisation layer
enforces.

## Goal

A player installs one mod, hosts from the main menu, and a friend joins and picks a team that nobody
else has taken. From then on both play one career together — the same calendar, the same race, the same
results — without either needing to understand a single thing about how it works.

Practical targets, in the order they matter:

1. **One race, fully synced.** 22 cars, one shared clock, from lights out to the flag.
2. **A shared career.** Contracts, car development and finances agreed between the machines.
3. **Easy install.** The client is one `.pak` file. The host needs one extra executable. Nothing else.
4. **Launch from the game.** Hosting starts from the main menu, in the game's own UI, not from a
   command line.

## How

### The stack

```
host game ── coop_agent.dll (injected) ──┐
                                         ├── coop-link (one per machine) ── TCP ── coop-supervisor (relay)
client game ── coop_agent.dll (injected) ┘
```

| Component | Role |
|---|---|
| `coop_agent.dll` | Injected into the game. A 5-byte hook on the game thread applies state changes from a mailbox, and it serves a small HTTP endpoint the in-game menu uses. |
| `coop-link` | One per machine. Reads the local simulation, publishes it, receives the other machine's, and decides what to correct. |
| `coop-supervisor` | The relay between the two machines. |
| `CoopBridge_P.pak` | The Gameface UI: a Multiplayer entry in the main menu, a lobby screen, and a bridge script. |

Everything is Rust except the UI, which is JavaScript in the game's own module system.

### Why it is hard: the simulation has no fixed timestep

This is the single most important fact in the project, and it was learned the hard way over four
failed races.

```
Config/DefaultEngine.json
  bSubstepping        = False
  MaxPhysicsDeltaTime = 0.033333
```

`bSubstepping=False` means the simulation consumes a **variable wall-clock delta**, up to 33 ms per
frame. There is no tick number, no fixed step, no way for two machines to be on the same step.

The consequence is that overwriting state cannot work. You write a car's distance, and the simulation
has already advanced past your value by an amount proportional to the frame-time difference between
the two machines. In the last session **1.88 million field writes were applied while the distance delta
grew to 62 km.** The writes landed. The simulation simply stepped past them.

So correction is not "copy the host's value". It is a *delta, scaled by the gap between the two
simulations' clocks*, chosen so the receiving simulation converges on the host's trajectory instead of
snapping to a position it will immediately leave. That model lives in `coop-proto::correct`.

The second trap: **`race_time` is not a clock.** It is `max(lap_time)` across all cars, so it freezes
whenever no car completes a lap — exactly when you need it to keep ticking. `SIM_TIME`, at `0x99AF1C`
in this build, is the real clock.

### Why the UI is a patch, not an overlay

The front-end is Gameface (Cohtml) running plain AMD modules with preact. `FrontendApp.js` renders
`FrontendModule`, which renders `FrontendData.screensLUT[screenID].component`. Navigation goes through
`FrontendNavigation.set`, which dispatches `onChange` *and* sends the `FrontendScreenChanged` engine
event.

So the multiplayer screen is registered in `screensLUT` and rendered like any built-in screen, and the
menu entry is a real `MenuButton` inserted into the real main menu. It inherits the game's layout,
focus order, styling and input handling because it *is* the game's UI. A floating panel in the corner
would have been less work and would have looked and felt wrong.

One consequence worth knowing: the Gameface context has `fetch` and `XMLHttpRequest` but no
`child_process`. The UI cannot start a process. That is why hosting is an HTTP call to the already
resident agent rather than the menu launching an executable — and why the client can be a single `.pak`
with no extra program.

## Progress

Measured, not estimated. "Works" means a test proves it, "not wired" means the code exists and is
tested in isolation but nothing calls it yet.

### Working

| | Evidence |
|---|---|
| Agent injects and hooks the game thread cleanly | First clean install in the project's history, after finding that `VirtualAlloc(NULL, …)` returns ~2 TB away and truncated every jump. Fires 25–60×/s, game stays responsive. |
| Writes are applied on the game thread | A two-buffer mailbox. Writing from the link process crashed the game (`EXCEPTION_ACCESS_VIOLATION reading 0xF3C`) because the game thread was mid-read. |
| Car position, speed and heading are read and published | Real values from a live race. |
| Tyre temperatures and wear are read | Surface 107–135 °C, carcass 99–121 °C, condition 0.90–0.92, `out-of-range values: 0 of 88`. Sync is staged `observe` → `wear` → `full`; the default writes nothing. |
| Correction model | Time-gap scaled, wraparound-safe heading, converges over 400 frames in test. Wired into `coop-link`. |
| Race-control messages reach the client | 1-byte width safety, tag collision (`TAG_READY` vs `TAG_AGENT` both 6) fixed. |
| Main-menu integration | 29 checks against a copy of the game's own render tree: entry lands in `MainMenu_rightBox`, carousel intact, left column untouched. |
| Pak deploy and preflight | `deploy_pak.py` refuses success unless the installed pak is byte-identical to the build. `preflight.py` separates wiring from flow. |

### Not wired yet

| | State |
|---|---|
| **Start hold** (wait for both players before the lights) | Written and tested in `coop-link/src/starthold.rs` — 8 tests. **Nothing calls it.** This is the "lights hold for player 2" behaviour, and it does not exist in a running game. |
| **Authorisation** (who may write what, per mode) | Written and tested in `coop-auth` — 13 tests, covering the three modes. **Nothing calls it.** The rules are decided; nothing enforces them. |
| **Race sync** | The correction model is wired, but no session has converged. See below. |

### Not working

- **Location sync does not hold.** Cars diverge during a race.
- **Car damage is not synced.**
- **Pit stops are not synced.** The whole grid has pitted on one machine and not the other.
- **Restarting a red-flagged session is broken.** Cars have left the pits while one side was not ready.
- **The lobby screen has never been rendered on screen.** Its button and layout are verified only
  against a copy of the game's render tree, never against the game.

### By the numbers

| | |
|---|---|
| Rust | 12,788 lines |
| Tests | 99 Rust, 29 UI — all passing |
| Tests that cannot run without a game | 3, in `coop-mem --test loop_stub_volume`; that binary faults off-game by design |
| Live races attempted | 4; none synchronised |
| Playthroughs verified | 0 |

## Build

Rust 1.75+. No game and no second machine are needed to build or test.

```sh
cd mod/coop-rs
cargo test --workspace        # 99 tests
cargo build --release
```

| Task | Command |
|---|---|
| UI integration test | `node mod/tools/menu_test.js` |
| Is the installed UI pak the one you built? | `python mod/tools/deploy_pak.py --check` |
| Is everything wired for a session? | `python mod/tools/preflight.py` |

Find your install automatically (Steam library folders are read, so any drive works), or point at it:

```sh
python mod/tools/deploy_pak.py --check          # finds the game via Steam
set COOP_GAME_DIR=D:\Games\F1 Manager 2024       # or say where it is
```

## Plan

Ordered by dependency, because almost everything waits on the clock being right.

1. **Confirm the multiplayer entry actually renders on screen.** One game launch. It has never been
   seen, and until it has, the UI is a claim rather than a fact.
2. **Wire the start hold.** The code and its tests exist; the publisher needs to call it. This is the
   specific thing that "lights must hold for player 2" requires.
3. **Give both simulations one clock.** Publish `SIM_TIME` with every frame and correct against the
   gap. Then one race, start to finish, with no divergence.
4. **Damage, then pit stops, then grid state** — in that order, because all three depend on the clock.
5. **Wire the authorisation layer** and finish the career screens: contracts, car development,
   facilities, finances.
6. **Reduce the install.** Collapse the three config files into one, and move the relay onto an agent
   thread so `coop-supervisor.exe` disappears.

## Problems worth reading

If you are picking this up, these are the traps that cost the most time. Each is a measured cause, not
a theory.

| Symptom | Real cause |
|---|---|
| "Trampoline fault / collapsed stack" on hook install | `VirtualAlloc(NULL, …)` returns ~2 TB from the module, so every `E9 rel32` jump was truncated. Allocate within ±2 GB. |
| Game crashed when the link wrote state | The game thread was mid-read. Writes must happen on the game thread, from a mailbox. |
| Race diverged while thousands of writes "succeeded" | No fixed timestep. Writes were applied and immediately stepped past. Correct by delta, not by value. |
| `Ready` decoded as `Agent` | Both tags were value 6. |
| Sync appeared to work, then fell 62 km behind | `race_time` is `max(lap_time)` and freezes. Use `SIM_TIME`. |
| Multiplayer button "did not appear" (twice) | Once, the installed `.pak` was two days older than the build — the game holds the file open, so the copy had silently failed and **no code had ever run**. Now `deploy_pak.py` verifies the installed pak against the build. |
| Same symptom, second cause | The UI required `project/apps/FrontendRouter`, which does not exist in the export, so RequireJS failed the *entire* module load and returned before patching anything. Also: `preact.h` deletes `attributes.children`, and `MainMenu` is a decorator whose inner class is what draws the menu. |

The lesson, recorded everywhere in `mod/docs/`: **verify the premise before building on it.** Every one
of the four failed races added replicated fields while never establishing that the simulation could be
made to agree.

## Where things are

| Path | What |
|---|---|
| `mod/coop-rs/` | The Rust workspace: agent, link, relay, protocol, authorisation |
| `mod/pak/` | The Gameface UI override |
| `mod/tools/` | Preflight gate, pak deploy/verify, hot-reload server, watchers |
| `mod/client/` | What gets copied onto the second machine |
| `mod/docs/` | The design record: what failed, why, and what the export proves |
| `mod/methodology/` | How this was developed with two machines. **Not part of the mod.** |
| `games/f1-manager-2024/` | Write-up in the Co-op Anything directory format |

Start with [`mod/docs/COOP-PLAN.md`](mod/docs/COOP-PLAN.md) for the design, and
[`mod/docs/NOTES.md`](mod/docs/NOTES.md) for the raw record.

## Credit

Built on the [Co-op Anything](https://github.com/cranebeaker/coop-anything) playbook, which this repo
is a fork of. The hub's own content is intact here — `playbook/`, `games/`, `tools/`, `DIRECTORY.md` —
because the playbook is how the work is being done. The upstream README is preserved at
[`mod/docs/upstream-readme.md`](mod/docs/upstream-readme.md).

Licence: MIT, as the hub.
