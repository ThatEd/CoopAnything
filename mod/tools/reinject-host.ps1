# Reinject the agent after a game restart, on this machine.
#
# Run after starting F1 Manager 24. Picks the largest F1Manager24 process (the real game, not the
# launcher), injects the newly built agent, and reports whether the hook installed and which tyre mode
# is armed. Idempotent: safe to run twice.
#
# Why this exists: a running process keeps the agent build it loaded, so a new DLL needs a fresh process.
# Re-injecting by path into a process that already has that path loaded is a silent no-op - LoadLibraryW
# returns the existing handle - and that has cost two test cycles.

param(
    # The game's Win64 folder, if the automatic lookup picks the wrong install.
    [string]$GameDir = $env:COOP_GAME_DIR
)

$d = $null
if ($GameDir) {
    $d = Join-Path $GameDir 'F1Manager24\Binaries\Win64'
} else {
    # Resolve through Steam rather than assuming a drive letter: same lookup the Python tools use.
    $found = & python (Join-Path $PSScriptRoot 'gamedir.py') 2>$null
    if ($LASTEXITCODE -eq 0 -and $found) {
        $d = Join-Path $found.Trim() 'F1Manager24\Binaries\Win64'
    }
}
if (-not $d -or -not (Test-Path $d)) {
    Write-Output 'GAME NOT FOUND - set COOP_GAME_DIR to the install folder (the one containing F1Manager24)'
    exit 1
}
Set-Location $d

$game = Get-Process F1Manager24 -ErrorAction SilentlyContinue |
        Sort-Object WorkingSet64 -Descending | Select-Object -First 1
if (-not $game) {
    Write-Output 'NO GAME PROCESS - start F1 Manager 24 first'
    exit 1
}

# Opt the two optional behaviours on explicitly, so a fresh process is fully armed.
Set-Content -Path "$d\coop_hook.enable" -Value 'enable'
if (-not (Test-Path "$d\coop_tyres.txt")) {
    Set-Content -Path "$d\coop_tyres.txt" -Value 'observe'
}

$dll = Get-Item "$d\coop_agent.dll"
$hash = (Get-FileHash $dll.FullName -Algorithm SHA256).Hash.Substring(0, 16)
Write-Output ("dll on disk : {0} bytes  {1}" -f $dll.Length, $hash)
Write-Output ("tyre mode   : {0}" -f (Get-Content "$d\coop_tyres.txt"))
Write-Output ("game pid    : {0}" -f $game.Id)

Remove-Item "$d\coop_agent.log" -Force -ErrorAction SilentlyContinue

& "$d\coop-inject.exe" --pid $game.Id --dll $dll.FullName 2>&1 | Select-Object -First 3
Start-Sleep -Seconds 5

Write-Output ''
Write-Output '=== agent log ==='
if (Test-Path "$d\coop_agent.log") {
    Get-Content "$d\coop_agent.log"
} else {
    Write-Output 'NO LOG - injection did not take'
    exit 1
}

Write-Output ''
$installed = Select-String -Path "$d\coop_agent.log" -Pattern 'hook installed' -Quiet
if ($installed) {
    Write-Output 'OK: hook installed. Start or enter a race session; the hook fires once the sim runs.'
    Write-Output 'Then start the publisher on the host side and the client on the other machine.'
} else {
    Write-Output 'PROBLEM: hook did not install - read the log above.'
}
