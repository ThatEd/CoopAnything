# Diagnose the client machine's sync state in one pass.
#
# Run through `remote.py runfile`. Reads only; changes nothing.

$d = 'E:\SteamLibrary\steamapps\common\F1 Manager 2024\F1Manager24\Binaries\Win64'
Set-Location $d

Write-Output '=== processes ==='
Get-Process F1Manager24, coop-link -ErrorAction SilentlyContinue |
    Select-Object Id, ProcessName, Responding,
        @{n = 'WS_MB'; e = { [int]($_.WorkingSet64 / 1MB) } } |
    Format-Table -AutoSize | Out-String -Width 200

Write-Output '=== agent injected? (coop_agent.log) ==='
if (Test-Path "$d\coop_agent.log") {
    Get-Content "$d\coop_agent.log" -Tail 14
} else {
    Write-Output 'MISSING - coop_agent.dll is not running in the game'
}

Write-Output ''
Write-Output '=== hook enabled? ==='
Write-Output ("coop_hook.enable exists: {0}" -f (Test-Path "$d\coop_hook.enable"))

Write-Output ''
Write-Output '=== client-run.log ==='
if (Test-Path "$d\client-run.log") { Get-Content "$d\client-run.log" -Tail 10 }
else { Write-Output 'missing' }

Write-Output ''
Write-Output '=== client-run.log.err ==='
if (Test-Path "$d\client-run.log.err") { Get-Content "$d\client-run.log.err" -Tail 12 }
else { Write-Output 'missing' }

Write-Output ''
Write-Output '=== inject.log ==='
if (Test-Path "$d\inject.log") { Get-Content "$d\inject.log" -Tail 10 }
else { Write-Output 'missing' }

Write-Output ''
Write-Output '=== is the write port listening? ==='
$conn = Get-NetTCPConnection -LocalPort 45991 -State Listen -ErrorAction SilentlyContinue
if ($conn) {
    $conn | Select-Object LocalAddress, LocalPort, OwningProcess | Format-Table -AutoSize | Out-String -Width 120
} else {
    Write-Output 'port 45991 NOT listening - the agent is not in the game (or did not start its listener)'
}
