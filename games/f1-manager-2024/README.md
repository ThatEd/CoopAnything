# F1 Manager 2024 (2024)

| | |
|---|---|
| Engine | Unreal Engine 5.1 (Frontier Developments) |
| Versions tested | Steam, appid 2591280, build 17356935 (exe SHA-256 `7ffe404f…b2050f6`) |
| Projects | This fork's `mod/` — no other co-op project found |
| QA level | none |
| Last updated | 2026-10-27 |

## Where things stand

Partly working, not playable. Car positions **do** synchronise and were measured at
`maxDistDelta=0.00` in three separate live sessions, with all 22 cars tracked and the lap order correct
— then the two simulations come apart as the clocks drift, once by 62 km while 1.88 M writes were being
applied. Nothing else is synced: not damage, not pit stops, not race control. Four live race sessions
were attempted and none held sync to the flag. Everything below is measured from the game, not inferred.

## Existing work

Nothing. **Searched:** GitHub repositories and code search for "F1 Manager 2024 coop", "F1Manager24
coop", "F1 Manager 2024 multiplayer", "F1 Manager coop multiplayer"; Nexus Mods; ModDB; Steam guides.
What exists is save editing and telemetry — DatabaseEditor (save editor), F1Manager2024Logger,
trainers, setup tools. The modding scene is save-editing, not runtime.

There is no netcode to build on: the game is single-player with no multiplayer layer, so co-op has to
be built from the outside.

## How the game works (for co-op)

**Simulation.** The decisive fact is in `Config/DefaultEngine.json`:

```
bSubstepping        = False
MaxPhysicsDeltaTime = 0.033333
MaxSubsteps         = 6
```

No fixed timestep. Each machine advances by its own frame time, up to 33 ms, so they are never at the
same step. Per-field writing therefore cannot converge: in the last session 1.88 M writes were applied
while the distance delta grew to 62 km. Correction has to scale with the *time gap* between the two
simulations rather than overwrite state.

**Clock.** `race_time` is `max(lap_time)` across cars and freezes when no car completes a lap, so it is
not a usable clock. `SIM_TIME` (at `0x99AF1C` in this build) is.

**Save format.** GVAS wrapping a zlib-compressed SQLite database, already reverse engineered. The schema
is in the export as `Content/Database/SavePS.json`, ~40 tables of the game's own persistence layer.
Facts that matter for co-op: `Save_CarConfig` is keyed `TeamID`+`LoadoutID`; `Save_DriverCommands` and
`Save_PlayerStrategyStints` by `LoadoutID`; `Save_CarTyreAllocation` by `CarID`; `Buildings_HQ` by
`TeamID`. So "my side of the garage" is `LoadoutID` and "facilities" is `TeamID`.
`Save_RaceSimCars_Parts` carries six named damage levels, so damage is persisted, not only
event-driven. `Player` has exactly one row, so two players cannot both be "the Player" — versus mode is
a compromise on that.

**UI.** Gameface (Cohtml), plain AMD modules with preact. `FrontendApp.js` renders `FrontendModule`,
which renders `FrontendData.screensLUT[screenID].component`. Navigation goes through
`FrontendNavigation.set`, which dispatches `onChange` and sends the `FrontendScreenChanged` engine
event; a raw `engine.trigger` does neither. There is no router module — `screensLUT` is the seam. The
front-end context has `fetch` and `XMLHttpRequest` but **no** `child_process`, so the UI cannot start a
process; hosting has to be done by the injected agent.

**Scripting.** UE4SS works for Lua and can reflect on the game (enumerate `GUObjectArray`, read fields,
fire UI events) but cannot write reliably enough for per-frame simulation sync, which is why a native
hook exists.

## What we changed

Each item is a bug with a measured cause.

**Hook install produced a "trampoline fault / collapsed stack".** Root cause: `VirtualAlloc(NULL, …)`
returns an address roughly 2 TB from the module on this machine, so every `E9 rel32` jump was
truncated. Fixed by allocating within ±2 GB of the target. This was the first clean hook install in the
project.

**Trampoline leaked 8 bytes of stack.** A `jz` skipped a `pop rax`, leaving `rsp` low on every skipped
path.

**Writes from the link process crashed the game** (`EXCEPTION_ACCESS_VIOLATION reading 0xF3C`). The game
thread was mid-consumption of those fields when an external thread changed them. Fixed with a two-buffer
write mailbox: the link stages writes and the game thread applies them.

**`TAG_READY` collided with `TAG_AGENT`.** Both were value 6, so `Ready` decoded as `Agent`. Renumbered
to 10, with a test asserting uniqueness.

**Tyre sync had no safety.** Implemented in three explicit stages — `observe` (default, writes
nothing), `wear`, `full` — with range validation, `NaN` rejection and skip-not-clamp semantics.

**Correction did not converge.** Replaced with a time-gap-scaled model in `coop-proto::correct`, with
wraparound-safe heading handling and a 400-frame convergence test.

**Multiplayer entry did not appear in the main menu.** Two faults. First, the pak in both game installs
was two days old (17,602 bytes against a 50,017-byte build); the game holds the pak open, so the copy
had silently failed and nothing written had ever run. Second, the integration required
`project/apps/FrontendRouter`, which does not exist in the export, so RequireJS failed the entire module
load and bailed out before patching anything. The screen is now registered through `screensLUT`. Also
established that `preact.h` deletes `attributes.children` (children live on `vnode.children`), that
`EMPTY_CHILDREN` is shared so trees must be rebuilt rather than mutated, and that `MainMenu` is a
`Focusable.decorate` wrapper whose inner class is what actually draws the menu.

**Three game modes designed against the save schema rather than guessed.** `Save_CarConfig` is keyed
`TeamID`+`LoadoutID`, so a player's cars are a loadout and a team's facilities are a team; that is what
shared-team mode means concretely. The rules are written and tested in `coop-auth`. **They are not
called by anything yet**, so no mode is enforced in a running game.

## QA

```sh
cd mod/coop-rs && cargo test --workspace     # 99 tests, no game and no second machine needed
node mod/tools/menu_test.js                  # 29 checks
python mod/tools/deploy_pak.py --check       # installed UI pak matches the build
python mod/tools/preflight.py                # session wiring
```

None of these require a second PC. `preflight.py` checks one only if `COOP_REMOTE_SHARE` is set;
otherwise those checks report `SKIP`. Running two machines was this project's development method and
lives in `mod/methodology/`, not in the mod.

`deploy_pak.py` finds the game through Steam's library list, so no drive letter has to be assumed;
`COOP_GAME_DIR` overrides it. The UI test's module-id check needs the game's UI export, which is not
ours to redistribute, so it is skipped unless `COOP_UI_EXPORT` points at one.

**What the tests do not prove.** Two pieces of code pass their tests while nothing calls them, and that
distinction matters more than the pass count:

- `coop-link/src/starthold.rs` — 8 tests for the race-start hold, wired into no caller.
- `coop-auth` — 13 tests for the three modes' write rules, called from nowhere.

Both are correct in isolation and absent from a running game. The correction model is the one large
component that is genuinely plumbed through `coop-link` into the agent.

Last suite result: **99 Rust tests and 29 UI checks passing, 2026-10-27.** A second test binary,
`coop-mem --test loop_stub_volume`, holds 3 more tests and faults without a running game, because it
drives 600 statements through the loop stub against the game's own SQLite. Those 3 are not counted
above and have not been shown to pass.

The UI checks assert that every module id the integration asks for exists in the export (this would have
caught the missing router), that the entry lands as the first child of `MainMenu_rightBox` with the
carousel intact, that the left column is untouched, that repeat renders do not stack entries, and that
the screen renders with all ten real `EF1Team` ids plus a back button and both start paths.

QA level is **none** because nothing has been verified in a playthrough. The tests prove what can be
proven without the game; they do not prove sync.

## How to play

You cannot, yet. When there is something to try it will be a release here with an installer and a
one-click host and join.

## Known issues

- Sync does not survive a race. Positions agree while the clocks are in step (measured
  `maxDistDelta=0.00`), then diverge as they drift.
- The race start is not held for both players on all counts; restarting a red-flagged session has left
  cars leaving the pits while one side was not ready.
- Car damage is not synchronised.
- The entire grid can pit on one machine and not the other.
- The lobby screen has not been rendered on screen yet. Its button placement and styling are verified
  only against a copy of the game's render tree.

## Next steps

1. Launch the game, confirm the multiplayer entry renders, then fix placement and focus from what is
   actually on screen.
2. Wire the start hold (`coop-link/src/starthold.rs`, 8 tests) into the publisher so the lights wait for
   both players.
3. Give the client a shared clock: publish `SIM_TIME` with each frame and correct against the gap rather
   than writing absolute positions.
4. Then damage, pit stops and grid state, in that order — all depend on the clock being right.
5. Collapse the three config files into one, and host the relay on an agent thread so
   `coop-supervisor.exe` disappears from the player's install.
