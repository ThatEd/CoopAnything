# Client side — F1 Manager 2024 co-op test link

Everything here is copied into the client's game folder:

```
<game dir>\F1Manager24\Binaries\Win64\
```

That is the folder holding `F1Manager24.exe`. The share was **read-only** for this machine (write
probe denied, even though the NTFS ACL grants Modify — the SMB share itself is read-only), so these
files are staged here to be copied across rather than written directly.

## What the files are

| file | what it does |
|---|---|
| `coop-link.exe` | the network end. `--role client` receives snapshots; `--role host` reads a local game and publishes them |
| `coop-supervisor.exe` | the relay. Keeps `Host` and `Client` peers and forwards `Host` frames to every `Client` |
| `coop-inject.exe` | injects the agent DLL into a running game by pid |
| `coop_agent.dll` | in-process agent: locates the RaceSimManager, reports it, and can write a dump on request |
| `coop_supervisor.txt` | one line, `host:port`. The injected agent reads it, which is how it learns to talk to the **host** machine instead of its own loopback |

Nothing here modifies the game's files or saves. The agent is only mapped into a running process, and
that process returns to normal when it exits.

## Join with a hotkey

Run `coop-hotkey.cmd` once on the client and leave it minimised. Then:

| key | action |
|---|---|
| **Ctrl+Alt+J** | join the host session |
| Ctrl+Alt+Shift+J | same, if the first is taken by another program |

Pressing it again stops the previous link and starts a fresh one, so a join can simply be retried.
Everything is logged to `coop_link.log` in the game folder ? the link runs in its own console window,
so without the file a failed join would be invisible.

This is not a Lua/UE4SS keybind. The client install has no UE4SS, and installing a loader into a second
machine's game just to read a keystroke is a poor trade. `coop-keys.exe` registers a Windows global
hotkey, so it works while the game has focus, and it does its work in its own process ? the game never
sees it.

## Setup on the client

Copy the files in, then confirm the address file points at the host:

```
type coop_supervisor.txt
:: expected: <host-ip>:45990
```

## Verify the network path first — no race needed

On the host machine, in `coop-rs`:

```
coop-supervisor.exe --bind 0.0.0.0
coop-link.exe --role host --fake --supervisor 127.0.0.1:45990 --hz 20
```

On the client machine, in the game folder:

```
coop-link.exe --role client --supervisor <host-ip>:45990
```

The client should print, every two seconds:

```
[client] 40 snapshots, 22 cars, sim clock 6.1, worst gap 75 ms
```

and write `coop_shadow.txt` beside itself, listing 22 cars with rising distances. That proves the
firewall, the relay, the framing and the client without touching the game.

Then check the agent reaches the host supervisor. On the client:

```
coop-inject.exe --pid <F1Manager24 pid> --dll "%~dp0coop_agent.dll"
```

On the host supervisor you should see:

```
[supervisor] <client-ip>:<port> connected
[supervisor] agent in <pid>, rsm=0x...
```

## Firewall

The host must allow inbound TCP 45990. Windows prompts the first time `coop-supervisor.exe` binds
`0.0.0.0`; allow it on the private network.

## With a live race instead of `--fake`

On the host:

```
coop-link.exe --role host --supervisor 127.0.0.1:45990 --pid <host game pid> --hz 20
```

The host discovers the RaceSimManager itself, so no address is passed. If the game is at the menu it
reports `none looked live (menu?)` and keeps trying, which is correct: it refuses to guess rather than
read 10 MB of unrelated memory.

## Session transfer (no file copying)

The host can ship its save to the client over the same connection, so a co-op session does not need
anyone to move files around:

```
:: on the host
coop-link.exe --role host --supervisor 127.0.0.1:45990 --pid <host game pid> --send-save "<save file>"
```

The client receives it, verifies the hash, and writes it into `%LOCALAPPDATA%\F1Manager24\Saved\SaveGames`.
It prints one of:

```
[client] session offer: save168.sav, 2.80 MB, hash 0xD2944740
[client] session saved to ...\save168.sav (2936598 bytes, hash verified)
```

or a refusal, such as `session hash mismatch (got ..., want ...); not written`. A partial transfer is
never written: the receiver holds the bytes in memory and only writes once the hash matches, because a
truncated `.sav` in the game's save folder would be offered by the game's own load menu.

The payload is opaque. The game compresses saves with frontier's own compressor (`Oodle`/`zstd` are
linked statically into the exe, and the file is ~7.99 bits/byte with no SQLite or zlib marker in it),
so nothing here parses or rebuilds it ? the bytes are carried exactly. Verified: a 2 936 598-byte save
arrived with a matching SHA-256, byte for byte.

`COOP_SAVE_DIR` overrides the destination, which is how the transfer is tested without touching the
game folder.

## What this is not yet

The client receives and stores the host's race state, and can now receive the session save; it does
**not** yet ask its game to load that save, and nothing is written into game memory. Both are buildable
without a format understanding: "load this slot" is a normal game function, and the live 22-car state
is already travelling over this link.
