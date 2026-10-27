$d = 'E:\SteamLibrary\steamapps\common\F1 Manager 2024\F1Manager24\Binaries\Win64'

Write-Output '=== full client log (head) ==='
$log = Join-Path $d 'client-run.log'
if (Test-Path $log) { Get-Content $log -TotalCount 12 }

Write-Output ''
Write-Output '=== elevation warning present? ==='
if (Test-Path $log) {
    $hit = Select-String -Path $log -Pattern 'integrity level|WARNING' -SimpleMatch
    if ($hit) { $hit | ForEach-Object { $_.Line } } else { Write-Output 'NO WARNING FOUND' }
}

Write-Output ''
Write-Output '=== game running elevated? ==='
$game = Get-Process F1Manager24 -ErrorAction SilentlyContinue |
        Sort-Object WorkingSet64 -Descending | Select-Object -First 1
if ($game) {
    Write-Output ("game pid " + $game.Id + "  " + [int]($game.WorkingSet64/1MB) + " MB")
    # A process started elevated cannot be opened for write by an unelevated process even with
    # VM_WRITE; OpenProcess itself fails or the write is denied with 5.
    try {
        $fs = [System.IO.File]::Open($game.Path, 'Open', 'Read', 'ReadWrite')
        $fs.Close()
        Write-Output 'game exe is NOT locked exclusively (weak signal)'
    } catch { }
} else {
    Write-Output 'no game process'
}

Write-Output ''
Write-Output '=== our own elevation ==='
Write-Output ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)

Write-Output ''
Write-Output '=== newest crash ==='
$crashes = Join-Path $env:LOCALAPPDATA 'F1Manager24\Saved\Crashes'
if (Test-Path $crashes) {
    $latest = Get-ChildItem $crashes -Directory | Sort-Object LastWriteTime -Descending | Select-Object -First 1
    Write-Output ($latest.Name + '  ' + $latest.LastWriteTime)
    $ctx = Join-Path $latest.FullName 'CrashContext.runtime-xml'
    if (Test-Path $ctx) {
        $xml = Get-Content $ctx -Raw
        $m = [regex]::Match($xml, '<ErrorMessage>(.*?)</ErrorMessage>', 'Singleline')
        if ($m.Success) { Write-Output ('ErrorMessage: ' + $m.Groups[1].Value) }
    }
}
