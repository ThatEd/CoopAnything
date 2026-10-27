# Confirm the client is running the current build and is talking to its agent.
#
# Run through `remote.py runfile`. Reads only.

$d = if ($env:COOP_CLIENT_GAME) { Join-Path $env:COOP_CLIENT_GAME 'F1Manager24\Binaries\Win64' } else { 'C:\Program Files (x86)\Steam\steamapps\common\F1 Manager 2024\F1Manager24\Binaries\Win64' }
Set-Location $d

Write-Output '=== coop-link.exe on disk (must match the host build) ==='
$item = Get-Item "$d\coop-link.exe" -ErrorAction SilentlyContinue
if ($item) {
    $hash = (Get-FileHash "$d\coop-link.exe" -Algorithm SHA256).Hash.Substring(0, 16)
    Write-Output ("{0} bytes  {1}  {2}" -f $item.Length, $item.LastWriteTime.ToString('yyyy-MM-dd HH:mm'), $hash)
} else {
    Write-Output 'MISSING'
}

Write-Output ''
Write-Output '=== coop_agent.dll on disk ==='
$agentItem = Get-Item "$d\coop_agent.dll" -ErrorAction SilentlyContinue
if ($agentItem) {
    $agentHash = (Get-FileHash "$d\coop_agent.dll" -Algorithm SHA256).Hash.Substring(0, 16)
    Write-Output ("{0} bytes  {1}  {2}" -f $agentItem.Length, $agentItem.LastWriteTime.ToString('yyyy-MM-dd HH:mm'), $agentHash)
} else {
    Write-Output 'MISSING'
}

Write-Output ''
Write-Output '=== client-run.log (all lines) ==='
if (Test-Path "$d\client-run.log") {
    Get-Content "$d\client-run.log"
} else {
    Write-Output 'missing'
}

Write-Output ''
Write-Output '=== client-run.log.err (tail) ==='
if (Test-Path "$d\client-run.log.err") {
    Get-Content "$d\client-run.log.err" -Tail 6
} else {
    Write-Output 'missing'
}
