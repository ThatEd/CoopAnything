# Development method

This folder is **not part of the mod.** It is how this work was developed, kept separate so it is
clear what a player needs and what only a developer does.

The distinction matters for two reasons. First, someone building or playing the mod should not have to
understand a two-machine test rig to make sense of the repository. Second, none of it should leak into
what ships: no drive letters, no network shares, no assumption that a second PC exists.

## What was done with two machines

Reverse engineering a game's simulation needs a second copy running to compare against. The method
here was: run the host, drive a client, and diff what each machine believes. That is why so much of
this folder exists.

| file | what it does |
|---|---|
| `tools/remote.py` | run a command on the other PC over an SMB share and read its output back |
| `scripts/attach-client.ps1` | attach the sync client on the second machine to its own game and the host's relay |
| `scripts/diag-*.ps1` | one question each: is the agent there, is the link up, are frames flowing, is elevation right |
| `scripts/push-save.ps1` | copy a save to the second machine so both load the same career |
| `scripts/run/start-agent.*` | start the agent on the second machine from inside the game |

### Why a share and a file channel

The second machine had SMB open and nothing else: no SSH, no WinRM, no usable port. A writable share
was enough to run a job, read its output and exit code, and pull logs back. The protocol is dumb on
purpose — write `<id>.cmd`, poll for `<id>.done`, read `<id>.json` — because it had to work over a
channel that kept flapping.

## What this is not

Nothing here is required to build, test or run the mod:

```sh
cd mod/coop-rs && cargo test --workspace   # works with no second machine
node mod/tools/menu_test.js                # works with no second machine
python mod/tools/deploy_pak.py --check     # works with no second machine
```

`mod/tools/preflight.py` is the one place the two overlap, and it is explicit about it: with
`COOP_REMOTE_SHARE` unset the second-machine checks report `SKIP`; with it set they are real failures
if the machine is unreachable. Nothing else in `mod/` looks for a second machine.

## Using it

Set the share root and the client's install, then run a script:

```powershell
$env:COOP_REMOTE_SHARE = '\\OTHER-PC\e'          # the share root, not a drive letter
$env:COOP_CLIENT_GAME  = 'C:\Games\F1 Manager 2024'
$env:HOST_ADDR         = '192.0.2.10:45990'      # this machine's LAN address, as the client sees it

python methodology/tools/remote.py --help
```

`remote.py` resolves the release binaries the same way `mod/tools/` does — by picking the newest build
rather than the first directory that exists, because a stale in-tree build is exactly as "existing" as
a fresh one, and shipping one cost two test cycles.

## If you only have one machine

You can develop and test a great deal without any of this: the Rust tests, the UI test, the pak
deploy check and the preflight gate all run locally. What you cannot do is watch two simulations
diverge, which is the specific problem this mod exists to solve.

For that, two copies on one PC is often enough — see the hub's
[multi-instance testing](https://github.com/cranebeaker/coop-anything/blob/main/playbook/multi-instance-testing.md)
and [automated QA](https://github.com/cranebeaker/coop-anything/blob/main/playbook/automated-qa.md)
pages. It removes the share, the IP address and half the failure modes.
