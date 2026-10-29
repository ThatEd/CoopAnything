#SPAWN
# Attach the sync client on THIS machine to its own game and to the host's relay.
#
# The `#SPAWN` marker must be the very first line: the agent tests it with `^\s*#SPAWN`, anchored to
# the start of the file. With a comment block above it the marker never matches, the agent runs the
# script in the foreground, and the whole agent blocks on Start-Process.
#
# Two further constraints this script obeys:
#  1. The agent starts it detached rather than waiting, so the agent itself never blocks.
#  2. No redirecting a child's stdout/stderr to a path on the network share. Doing that makes
#     Start-Process block indefinitely. The child writes to a LOCAL path only.
#
# What changed and why
# --------------------
# The client no longer writes to its own sim. `coop-link` sends car frames to `coop_agent.dll` over
# loopback, and the agent's hook applies them **on the game thread**. Writing from the client process
# is what produced `EXCEPTION_ACCESS_VIOLATION reading address 0xF3C` on three separate tests: the
# game is midway through consuming those fields when an external thread changes them.
#
# So this script has one more job: make sure the agent is injected and the hook is enabled. Without
# both, `coop-link` stages frames and nothing ever applies - which looks exactly like "no sync".

param(
    # The host's relay address, e.g. 192.0.2.10:45990. Required: it differs per network, so it is not
    # committed. Pass -Supervisor explicitly or set HOST_ADDR.
    [string]$Supervisor = $env:HOST_ADDR,
    # The client's game install, if it is not in the default Steam location.
    [string]$GameDir = $env:COOP_CLIENT_GAME
)

$d = if ($GameDir) { Join-Path $GameDir 'F1Manager24\Binaries\Win64' } else { 'C:\Program Files (x86)\Steam\steamapps\common\F1 Manager 2024\F1Manager24\Binaries\Win64' }
if (-not (Test-Path $d)) {
    Write-Error "game binaries not found at $d - set COOP_CLIENT_GAME to the game install folder"
    exit 1
}
if (-not $Supervisor) {
    Write-Error "no host address - pass -Supervisor host:45990 or set HOST_ADDR"
    exit 1
}
Set-Location $d

$log = Join-Path $d 'client-run.log'
$err = Join-Path $d 'client-run.log.err'
$agentLog = Join-Path $d 'coop_agent.log'

Get-CimInstance Win32_Process -Filter "Name='coop-link.exe'" | ForEach-Object {
    Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue
}
Start-Sleep -Seconds 2

$game = Get-Process F1Manager24 -ErrorAction SilentlyContinue |
        Sort-Object WorkingSet64 -Descending | Select-Object -First 1
if (-not $game) {
    Set-Content -Path $log -Value 'NO GAME PROCESS'
    exit 1
}

# The hook is opt-in by design - a bad patch is an instant crash with no dump - so enable it here
# rather than relying on remembering.
Set-Content -Path (Join-Path $d 'coop_hook.enable') -Value 'enable the sim hook (see hook.rs)'

# Is the agent already inside the game? The DLL writes coop_agent.log next to the exe; a fresh write
# means it is resident. If not, inject it now.
function Agent-Fresh {
    if (-not (Test-Path $agentLog)) { return $false }
    $age = (Get-Date) - (Get-Item $agentLog).LastWriteTime
    return ($age.TotalSeconds -lt 20)
}

$injected = Agent-Fresh

if (-not $injected) {
    $injector = Join-Path $d 'coop-inject.exe'
    if (Test-Path $injector) {
        & $injector --pid "$($game.Id)" --dll (Join-Path $d 'coop_agent.dll') 2>&1 |
            Out-File -FilePath (Join-Path $d 'inject.log') -Encoding utf8
        Start-Sleep -Seconds 4
        $injected = Agent-Fresh
    } else {
        Set-Content -Path (Join-Path $d 'inject.log') -Value "NO INJECTOR at $injector"
    }
}

Remove-Item $log, $err -ErrorAction SilentlyContinue

$p = Start-Process -FilePath (Join-Path $d 'coop-link.exe') `
    -ArgumentList @('--role', 'client', '--pid', "$($game.Id)",
                    '--supervisor', $Supervisor, '--hz', '20') `
    -WorkingDirectory $d `
    -RedirectStandardOutput $log `
    -RedirectStandardError $err `
    -WindowStyle Hidden -PassThru

$agentState = if ($injected) { 'agent present' } else { 'AGENT NOT PRESENT - frames will not apply' }
Set-Content -Path (Join-Path $d 'client-attach.txt') `
    -Value ("attached {0} to game {1} at {2}; {3}" -f $p.Id, $game.Id, (Get-Date -Format o), $agentState)
