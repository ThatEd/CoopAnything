$d = if ($env:COOP_CLIENT_GAME) { Join-Path $env:COOP_CLIENT_GAME 'F1Manager24\Binaries\Win64' } else { 'C:\Program Files (x86)\Steam\steamapps\common\F1 Manager 2024\F1Manager24\Binaries\Win64' }

Write-Output '=== client game process ==='
Get-Process F1Manager24 -ErrorAction SilentlyContinue |
    Sort-Object WorkingSet64 -Descending |
    Select-Object Id, @{n='MB'; e={[int]($_.WorkingSet64/1MB)}}, Responding, MainWindowTitle |
    Format-Table -AutoSize | Out-String -Width 80

Write-Output '=== client coop-link ==='
Get-Process coop-link -ErrorAction SilentlyContinue |
    Select-Object Id, @{n='CPUs'; e={[math]::Round($_.CPU,1)}}, @{n='MB'; e={[int]($_.WorkingSet64/1MB)}} |
    Format-Table -AutoSize | Out-String -Width 60

Write-Output '=== newest crash report ==='
$crashes = Join-Path $env:LOCALAPPDATA 'F1Manager24\Saved\Crashes'
if (Test-Path $crashes) {
    $latest = Get-ChildItem $crashes -Directory | Sort-Object LastWriteTime -Descending | Select-Object -First 1
    if ($latest) {
        Write-Output ("dir: " + $latest.Name + "  " + $latest.LastWriteTime)
        $ctx = Join-Path $latest.FullName 'CrashContext.runtime-xml'
        if (Test-Path $ctx) {
            $xml = Get-Content $ctx -Raw
            foreach ($tag in 'ErrorMessage','CrashType','PCallStack') {
                $m = [regex]::Match($xml, "<$tag>(.*?)</$tag>", 'Singleline')
                if ($m.Success) {
                    $v = $m.Groups[1].Value
                    if ($v.Length -gt 400) { $v = $v.Substring(0, 400) }
                    Write-Output ("$tag : " + $v)
                }
            }
        }
    }
} else {
    Write-Output 'no crash folder'
}

Write-Output '=== client log tail ==='
$log = Join-Path $d 'client-run.log'
if (Test-Path $log) { Get-Content $log -Tail 6 }
