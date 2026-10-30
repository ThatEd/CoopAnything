# City Game Studio (2021)

| | |
|---|---|
| Engine | Godot 2.1.7 (developer's custom build: GodotSteam, GodotDiscord, error handler) |
| Versions tested | Steam build 25723493 (`appmanifest_726840.acf`), game v1.29.0-rc3, exe `CityGameStudio64.exe` SHA-256 `3eff9fb8…93e9427` |
| Projects | [City Game Studio Multiplayer](https://github.com/ThatEd/City-Game-Studio-Multiplayer) — competitive co-op; directory entry is `kind: mod`, `status: prototype` |
| QA level | `automated` — a scripted harness builds the mod, launches a host and a client, joins them, and runs an in-game assertion suite on both machines that checks shared state. The core flows were also driven by hand through the real UI (`--coop-remote`) and photographed; what is not covered is listed below. |
| Last updated | 2026-10-30 |

## QA

The co-op checks are assertions on the game's own models, not screenshots: a management sim's
screens show layout, not correctness, and a wrong number looks like a right one. `--coop-verify=N`
runs after the join settles and logs `verify PASS/FAIL <item> (<detail>)` for each check.

Suite, run 2026-10-28 on a **fresh** game, host `coop-highland` + client `coop-tester`:

    client: 25 passed, 0 failed   -> verify: ALL OK
    host:   25 passed, 0 failed   -> verify: ALL OK
    client join: studios=1 staff=1 consoles=0
    crashes in the run: 0

The same suite on the older mid-game save reports 26/24 because that save already has a company name.
Two long hand-driven sessions held both machines on the same week through several in-game years
(1976 -> 1977 -> 1978) with the clock at 6x.

What the client asserted (each a line in the log with the values): its own company has a name, an
office, and money; **the office is in a home building, has a desk with a computer on it, and the
player is seated at that desk**; no office of ours sits in a building we cannot work in; it owns no
console; its date, speed and pause match the world; the host's company appears as a rival under its
real name, with its 12 offices; the host's 14 released games are present and 14 publish offers are
listed, with a complete offer (`Thor's Journey 3`).

A client's first office is covered by four more cases (`--coop-officetest=`), each planting the bad
state right before the company is placed and each ending `verify: ALL OK`:

    unfit   a 400-area office in a theater      -> dropped, one usable office
    empty   a 400-area office with no furniture -> adopted and furnished (desk, computer, seat)
    full    every home building claimed         -> a new office built on an empty lot
    normal  fresh join                          -> one home office, furnished

A scripted playtest (`--coop-play=<seconds>`) drives both players through real actions and cross-checks
them: each machine sends the other a digest of what it has, and the other compares it against what it
*renders*. Run for **30 minutes with every action enabled**:

    host:   30 checks, 0 failures   -> play: ALL OK
    client: 30 checks, 2 failures   (both "date did not advance": the clock is frozen in an automated run)
    native crashes: none

It found four bugs the assertion suite had been blind to - inherited console customers (a client with no
console showing 3,000,000 server users), duplicate "Home" offices left as empty rooms, a client with
**0 developers** after restoring a company (staff re-seated into an office the duplicate clean-up then
closed), and a native crash from `Object.get()` in the new code. All four are fixed and now asserted:

    PASS this company has at least one developer (1 developer(s))
    PASS every employee works in an office that exists ()
    PASS only one home office, however many times this player joined (1 home office(s))
    PASS no office of ours is an empty shell ()
    PASS another company's console is not one this company publishes on ()
    PASS the server holds nobody else's users (0)

Transfers and the menu are covered by the same suite plus dedicated flags, each run as two live
clients and all ending `verify: ALL OK` on both machines:

    office  --coop-officetest=        unfit | empty | full | normal (see above)
    play    --coop-play=1800           both players act; each checks the other's digest every minute
    dump    --coop-dump=<seconds>      offices, servers and consoles of both sides, in full
    money   --coop-gift=money:100000   host -100000, client +100000
    tech    --coop-gift=tech:missing   a genre the host bought becomes known to the client
    ip      --coop-gift=ip:first       host gives up "Thor's Journey 3", client holds the IP
    menu    --coop-menutest            one Multiplayer button, host/join behind it, stacked, on screen
    ui      --coop-uitest              in-game button opens the panel; an offer is made and taken
                                       from the panel, and the money moves on both machines

Package side: `python -I tools/godotpck/run_tests.py` — builds packs in all three Godot formats,
extracts, patches, deletes, tampers with an MD5 on purpose; round-trips and edits `engine.cfb`; five
mutation twins; verifies all 6,869 entry MD5s against the retail pack and, with `GODOTPCK_FULL=1`,
rebuilds it and compares every entry. Last result `ALL OK`.

Relay side: `local/CGS/city-game-studio/server/run_tests.py`, 8 cases including a deliberately stalled
reader, `ALL OK`.

Driven by hand instead of by script (`--coop-remote`), a live client was taken through the retail UI one
press at a time with a screenshot after each: the game menu's project button, its `PopupMenu` submenu,
*Self publishing*, a genre card, a theme card, the game's own Start button, and the design step's Next.
That path is what found the last **0 developers** bug: a restored company record described offices that
had never been furnished and carried no staff, so the player arrived in an empty room. A restored player
now gets what a new one gets - `restore_from_world` furnishes the surviving office and seats the player
when the record holds nobody. Verified on a live join (`studios=1 staff=1`) and on the screen.

Two build guards came out of the same work: naming an engine class this build lacks
(`InputEventMouseButton`, `InputEvent`) is a *compile* error that silently disables the rest of a file,
and `Array.extend()` does not exist here.

A second hand-driven session ran the clock at 6x until **several in-game years** had passed on both
machines (1976 -> 1977 -> 1978, same week on each check), and took a project the whole way from the
project panel to HOLD. That is what found the release crash, the weekly server-user leak, the missing
host company name and the stale clock label - none of which the assertion suite could see on its own.

A third session covered the end of a session, which nothing before it had: the host was killed while a
client was playing, and the client was watched on screen rather than in the log. It showed the game's own
warning popup - *The host left the session* - with a Confirm button, and pressing it really did reach the
main menu (the suite looks the menu scene up rather than trusting a timer). The client's company was
saved and handed to the relay on the way out, and a **returning** player got it back: the same money
(299,706 against 299,704 stored), one office, one developer, and their office **re-interiorated from the
saved layout** - 4 pieces of furniture - on the new host's map, whose tile ids are not the old ones.

Not covered: releasing a game from the UI needs one more pass - the Release button fires but the project
stays in `HOLD`, and the confirmed-correct button has not been pressed on a live host since it was
identified. The publish-offer list, the console panel and the map remain unexercised by hand.

## Where things stand

A playable competitive co-op mod now exists as a working prototype, and it is **not** shippable
yet. One player hosts a save; a second instance joins over a relay, boots the host's world, and
plays their own company inside it. Both companies exist as genuine rivals on each other's machine,
with their own names, map offices and released games, and the companies are saved into the world so
a player who leaves is still there next session.

What is verified by a run (commands and logs in [NOTES.md](NOTES.md)): two instances join and boot
the host's 8.85 MB world; each sees the other as a competitor with the right name, offices and
games; a fresh company owns zero consoles; a player's titles appear in the game's own publish-offer
feed; time cannot be moved unilaterally by a client; players cannot buy each other out; a player
who leaves keeps their company in the save and gets it back on rejoin; **money, technology and IP
can be transferred between players, and both machines agree on the result**; the main menu now
has a working `Co-op: host a game` / `Co-op: join a game` pair, checked by reading the live controls
back (visible, on screen, not overlapping, press sets the role); and **when the host disappears, the
client is told, saves, shows a disconnect screen and returns to the main menu, and a returning player
is handed their company and its interiors back by the relay.**

The relay is covered by its own suite of **12 cases, all passing** (`python server/run_tests.py`),
which runs in seconds with no game: world delivery to a late joiner, join routing, clock fan-out,
company storage and replay, the host-left broadcast, and a slow reader surviving a multi-megabyte
transfer. Two of those cases changed contract this pass - a client's company is now *also* kept by the
relay, and its socket stays open for a moment after the host dies so it can hand that record over.

What is **not** verified: the in-game screens themselves, other than the disconnect flow. Everything
else above is measured from logs, not from looking at the publish-offer list, the rival list or the
console panel. Treat the UI as unconfirmed until someone plays it.

Evidence and method: [NOTES.md](NOTES.md), 1,200+ lines covering every pass, including the engine
faults that cost days.

## Existing work

Nothing. Searched on 2026-10-26: this repo's directory and game write-ups, GitHub repositories
and issues (`"city game studio"`, the developer's tracker), web search for mods and co-op, the
Steam store/workshop/discussions/news, ModDB, GameBanana's "CGS Hub", Skymods. The only
multiplayer artefacts are three feature requests in the developer's issue tracker:

- [#88](https://github.com/binogure-studio/city-game-studio-i18n/issues/88) (2019-03, 2
  comments). The developer answered that there is no network layer and that multiplayer would
  require changing the game from real time to turn based.
- [#803](https://github.com/binogure-studio/city-game-studio-i18n/issues/803) (2022-06, no
  answer).
- [#985](https://github.com/binogure-studio/city-game-studio-i18n/issues/985) (2026-06, no
  answer): a detailed request for either a shared studio or rival studios in one world.

## How the game works (for co-op)

- **Engine:** Godot 2.1.7, a patched build (upstream Godot 2.1 ends at 2.1.6). The exe carries
  GodotSteam (workshop, leaderboards, user stats), GodotDiscord and a custom error handler. The
  ENet multiplayer module is **not** compiled in; `StreamPeerTCP`, `TCPServer`,
  `PacketPeerUDP` and `StreamPeerSSL` are, so a transport can be written in GDScript.
- **Packaging:** one `data.pck` (376 MB, 6,869 files, pack format 0) with 925 compiled scripts
  (`.gde`), binary scenes (`RSRC` `.scn`), `.tex` textures, `.xl` translations and
  `engine.cfb`, the binary project settings.
  [tools/godotpck](https://github.com/ThatEd/City-Game-Studio-Multiplayer/blob/main/tools/godotpck/README.md) reads, extracts and patches the container;
  every entry MD5 in the shipped pack verifies and a rebuilt pack reproduces all of them.
- **Scripts: encrypted, then readable.** Godot 2.x encrypts exported scripts by default
  (`FileAccessEncrypted`: `GDEC`, mode, MD5 of the plaintext, u64 length, AES-256-ECB ciphertext
  padded to 16 bytes) with a 32-byte key compiled into the exe. Upstream Godot 2.1.6 source
  explains the whole format; the key was recovered from the exe by testing 32-byte windows
  against the header MD5 (142 s, one hit), and Godot RE Tools then decompiled **925 of 925**
  scripts with `--force-bytecode-version=2.1.3-stable`. The key is deliberately not in this
  repo; [NOTES.md](NOTES.md) records the method so it can be reproduced on your own copy.
- **Loading:** Godot 2 resolves every resource load through `remap/all` in `engine.cfb`
  (1,477 pairs, 925 of them scripts). The GDScript loader picks its mode from the *remapped*
  path's extension, which is why scenes say `res://scripts/main.gd` while the pack only holds
  `.gde`. Drop a pair with [tools/godotpck/ecfg.pyhttps://github.com/ThatEd/City-Game-Studio-Multiplayer/blob/main/tools/godotpck/ecfg.py) and the same
  scene loads a plaintext `.gd` you supply. That is the whole injection technique.
- **Mods are data only.** The developer ships a Mod Maker whose complete output is JSON for
  content types (competitors, conventions, difficulty, engines…), delivered through the Steam
  Workshop. There is no script-mod hook — so a co-op mod has to patch a copy of the pack.
- **Netcode:** none for players. The game does talk to the network as a client
  (`scripts/twitch-manager.gd`, `scripts/http/request.gd`, GodotSteam).
- **Clock:** `scripts/models/date.gd` is the whole simulation clock. It accumulates delta, and
  every 14 s scaled by speed it advances `_raw_date` by one week and queues `end_of_week`,
  `new_week`, `end_of_month`, `month`, `year` as signals, one per frame. Everything else hangs
  off those signals, so a host-authoritative clock plus replayed signals is the sync seam.
- **Saves:** `scripts/save/save-manager.gd`, threaded, under `user://`; state is serialised per
  model (`scripts/models/*.gd`), so a joining player can be served a save snapshot.
- **Reality of shipping:** the mod is a patcher that rebuilds the player's own `data.pck` with
  replaced/added `.gd` files and an edited `engine.cfb`. No game code or asset is redistributed.

## What we changed

Nothing in the game itself. Added:

- [tools/godotpck](https://github.com/ThatEd/City-Game-Studio-Multiplayer/blob/main/tools/godotpck/README.md) — reads, extracts and patches Godot packs in
  all three formats with MD5s recomputed (`pck.py`), and edits the Godot 2 settings blob that
  decides bytecode vs plaintext loading (`ecfg.py`). Both verified against this game's retail
  files, plus mutation twins.
- This write-up and [NOTES.md](NOTES.md), including the correction that the script encryption is
  stock Godot 2.x behaviour rather than a custom engine patch.
- A Godot section in [engine-families](../../playbook/engine-families.md) and a Godot recipe in
  [reverse-engineering](../../playbook/reverse-engineering.md).

The experiment that matters: a copy of the retail install, whose `data.pck` was rebuilt with
the decompiled `scripts/main.gd` (plus an `OS.set_window_title` marker), `scripts/main.gde`
deleted and the matching `remap/all` pair dropped, boots to the menu and shows the marker in the
window title. No game files were modified in the real install.

## How to play

Not yet - there is no installer. What exists: the main menu has a **Multiplayer** button that reveals
`Host a game` and `Join a game` (with an address box), and once a game is running another
**Multiplayer** button sits at the end of the game's own menu bar, opening a panel where money,
technology and the IP of a published game can be offered to the other player, and offers waiting for
you can be taken or refused. See `local/CGS/city-game-studio/build/build_mod.py` in the working area.

## Known issues

- **Seen, but not by its owner.** The panels have now been driven by hand and photographed: the game
  menu's project button, its submenu, the new-game form with genre and theme chosen, the design step,
  the rival list showing the host as `Highland Games`, and the client's home office with a worker at a
  desk. Fonts, theme and the text button among the game's icon buttons are confirmed on screen. What has
  not been looked at is the publish-offer list, the console panel and the map.
- **The game's Steam window covers the bottom of the form.** With no Steam running the client shows a
  "Steam not detected" `WindowDialog` that floats above every popup the game opens and hides the lower
  half of the new-game form. A player closes it with its own X; a test run has to do the same
  (`--coop-remote`, command `steam`).
- **A client's first office is placed by the mod, not chosen by the player.** It goes in an empty
  building of a home-capable type (or a new one is built on an empty lot when none is free), with a
  desk, a computer and the player seated at it. A player cannot pick the location themselves yet.
- **The date does not advance in an automated run** unless nothing is open. The game freezes its own
  clock whenever any popup is on top (`controllers/date.gd compute_state` freezes unless the game's main
  scene is on top), so the new-game tutorial, the inbox/annual report, a stray convention panel, and each
  step of a project all pause it. That is engine behaviour, not a co-op defect, and it is why an
  automated run reports `raw` stuck: one popup away from moving. Driving it by hand (`--coop-remote`,
  `tree` then `press`) runs the clock for real - two sessions reached several in-game years this way.
- **An intermittent join stall.** One run in about a dozen: the joiner sat at the main menu with the
  game's own "Loading mods" dialog up, its log ending at `inbox=65536` mid-stream while the process
  stayed responsive. The same arguments joined normally on the next attempt, with no mod script error
  written. Not diagnosed; the next step is a run that keeps the joiner's log from the first byte of the
  world.
- **"Host crashes when a player creates a game" - fixed.** Reproduced twice with a clean signature: the
  host died with `0xc0000005` at `0xae5633` **one second after the client's first project** appeared in
  its company record. It was not the release; it was the *announcement*. `company.sync_games()` passed
  `model.get_game_by_id(...)` straight into `announce_game()`, which reads `.id` off it without checking,
  and that lookup returns null when the catalog has no matching entry - a null dereference inside the
  engine, so nothing was logged. The lookup is now checked and a null is skipped. Replaying the same
  sequence (client creates a game through the real UI, host receives it) the host stayed up through
  DESIGN, DEVELOPMENT, POLISH and HOLD, with no crash in 20 minutes.
- **A client re-earned the host's server users every week - fixed.** Emptying the store once at join was
  not enough: `_update_platforms` runs on every `end_of_week`, recomputes userbase from supported
  platforms and pushes it into server usage, so the old "3 million server users" symptom came back one
  tick later (measured: 58,783 three minutes after a join that had logged the reset). A client that owns
  no console now holds its usage at zero for as long as that is true.
- **A fresh host had no company name - fixed.** `registered as host ()` meant a nameless rival in the
  other player's list. A New Game company is unnamed until the player types one; the host now takes the
  session name, as the client already did.
- **Pressing Release does not publish yet.** The button is visible, enabled and its handler fires, but
  the project stays in `HOLD` and the catalogue stays empty. Two things are now known: the button is a
  toggle (`indev-project-item._try_to_release` calls `game_controller.next_state` only when the button
  is *not* disabled), and the node every earlier attempt pressed was the **wrong one** - three nodes in
  the tree are called `release`, and the real, visible one is
  `game/GUI/IndevProject/panel/container/left/scroll/margin/list/main/container/container/left/container/release`
  at `(81,313)`, not the 48x48 icon at `(1307,180)`. A press on it does spend money, so the handler runs.
  Confirming `published=1` afterwards needs one more pass on a live host; it does not crash and does not
  affect the other player.
- **A client whose host dies is handled, and the relay is where that was broken.** The client is told,
  saves, shows the game's own warning popup and returns to the main menu. Getting there needed three
  relay fixes: it kept no copy of a player's company from after the host's last publish, it closed the
  client's socket in the same call that told it the host was gone (so the hand-over had nowhere to go),
  and the stored record used a different shape (`offices`/`layouts`) from the one the restore path read
  (`studios`), so a returning player's company was recognised and then discarded. A returning player now
  gets their money, office and the office's **interior** back, reopened on the new host's map.
- **Test runs end when a peer dies.** With `--coop-quit` set, a host that has heard nothing from its
  client for 45 s quits, and a client that loses its host quits - unless the disconnect was handled, in
  which case the run lingers long enough to see the screen and confirms the main menu scene is actually
  up before ending.
- **A dead socket is no longer polled.** After the host's socket closed, the client kept asking it for
  readable bytes and the engine raised an error on every call - 4,618 identical lines, 1.1 MB of log,
  between the disconnect screen appearing and Confirm being pressed. The poll now stops at a socket that
  is not connected. Same flow after the fix: 2 KB of errors, none repeated.
- **The game's own calendar throws on the way back to the menu.** `gui/calendar.gd:89` calls
  `date_model.month_progression()` unguarded and keeps processing during `main_menu()`, when the models
  are being reset. It happens with or without the mod and does not crash; recorded, not patched.
- **A client's visible clock could disagree with its own date model - fixed.** The HUD calendar repaints
  only on the `month` signal, while `set_raw_date()` (how the mod corrects a drifted client) emits
  nothing, so a live session showed a client whose model said `1978/01` and whose clock still read
  AUG 1977. Corrections now emit `month` across a boundary, and an assertion compares the rendered
  labels against the model.
- **Client consoles are half-built.** A host's console is registered on the client as a platform
  owned by that company, but the client's own game can still show it in places it should not, and
  clients logged into it were seen before the console model was excluded from the join.
- **A technology gift usually has nothing to move.** Measured, not assumed: at the same date both
  companies know the same 18 of 64 genres, and all 55 engine features are available to everyone.
  A gift only transfers something when the giver bought a genre ahead of the date; otherwise the
  mod says so ("already knows every genre we do") instead of pretending.
- **Offers are made from a panel now, but there is no screen for a *full* trade.** The in-game
  Multiplayer panel offers money, a technology or an IP and lists what is waiting for you; there is no
  counter-offer or negotiation, and the address for joining is typed into a box rather than
  discovered.
- The mod is not written to survive a game patch: every patched script must be re-applied against
  each new build, and a patch may re-encrypt the scripts and change the key.
- `print()` is disabled by the game's own config (`application/disable_stdout`), so mod logging and
  test markers go to files next to the executable.
- While a player is away the host must explicitly allow the world to continue without them; until
  then everyone is paused. That is deliberate, but it has no UI yet - only a flag.
- No automated suite covers the game side beyond the in-game `--coop-verify` assertions.
  `local/CGS/city-game-studio/server/run_tests.py` covers the relay only (8 cases, `ALL OK`).

## Next steps

1. Watch it: the publish-offer list, the console panel and map ownership are still unconfirmed by eye.
   Everything else on that list has now been driven by hand (`--coop-remote`) and photographed.
2. Let a player compose an offer for what the other side *asked* for (a counter-offer), and show who
   is connected without opening the panel.
3. A client-initiated *speed* change over the real handshake (pause is proven, speed is not).
4. One-click host and join, then a bot that drives a studio so regressions are automatable.
5. Catch the intermittent join stall with a joiner-side log that starts at the first byte of the world.
6. Owner decision on publishing the recovered key (currently `local/` only) and on opening a claim
   issue and PR from their account.
