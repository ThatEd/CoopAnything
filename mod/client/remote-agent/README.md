# Remote agent

Code execution and telemetry on the other PC, over the SMB share. No listening port, no firewall
rule, no service to install — and it works while the game runs.

## Why a file channel

The other machine has 445 open but no SSH, no WinRM, no open coop port. What it does have is a
**writable** share at `<share root>` (set `COOP_REMOTE_SHARE`). That is enough for everything this project needs:

- run a command there
- read its output and exit code
- pull log files and saves back
- push a pak or a binary across
- poll game state while the game is running

An SMB write is already an authenticated remote file operation; the "protocol" is just files with
names that carry the request id.

## Layout, on the share

```
<share root>\coop-agent\
    req\      <id>.cmd      a PowerShell script to run
    out\      <id>.out      combined stdout/stderr
    out\      <id>.json     exit code, host, timings, working set
    out\      <id>.done     marker; its presence means json+out are complete
    put\                    files the caller wants on that machine
    get\                    files that machine publishes for the caller
```

The `.done` marker written **last** is what makes a poll race-free: a reader never sees a half-written
`.out`. This is the same "wait for the whole frame" rule the wire protocol already uses.

## Running it there

`remote-agent\run\start-agent.cmd` must be started once on that machine (or placed in Startup). It
polls `req\` every second and executes each request in a child PowerShell.

Because the share is writable from here, I can place the agent and its launcher through the share.
**Starting it is the one step that needs either a person at that machine or an existing autostart
entry**, since a file share cannot launch a process by itself. Everything after that is unattended.

## Why not scheduled tasks over RPC

`schtasks /S <other-machine>` needs admin credentials on that machine, which I do not have and should
not ask for. The share is the access that already exists.

## Safety

The agent runs whatever `.cmd` files appear in `req\`. That is deliberate — it is a remote shell — but
it means the directory must stay local to this project and must not be world-writable. Requests are
moved to `done\` after execution so nothing replays on restart.
