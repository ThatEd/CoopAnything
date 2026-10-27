$d = 'E:\SteamLibrary\steamapps\common\F1 Manager 2024\F1Manager24\Binaries\Win64'

Write-Output '=== coop-link process ==='
Get-CimInstance Win32_Process -Filter "Name='coop-link.exe'" |
    Select-Object ProcessId, CreationDate, CommandLine |
    Format-List | Out-String -Width 200

Write-Output '=== binary on disk ==='
$exe = Join-Path $d 'coop-link.exe'
if (Test-Path $exe) {
    $f = Get-Item $exe
    Write-Output ("{0} bytes  {1}  sha256 {2}" -f $f.Length, $f.LastWriteTime,
        (Get-FileHash $exe -Algorithm SHA256).Hash.Substring(0,16))
}

Write-Output ''
Write-Output '=== full client log ==='
$log = Join-Path $d 'client-run.log'
if (Test-Path $log) {
    $f = Get-Item $log
    Write-Output ("log {0} bytes  {1}" -f $f.Length, $f.LastWriteTime)
    Get-Content $log -Tail 20
}

Write-Output ''
Write-Output '=== log err tail ==='
$err = Join-Path $d 'client-run.log.err'
if (Test-Path $err) {
    $f = Get-Item $err
    Write-Output ("err {0} bytes  {1}" -f $f.Length, $f.LastWriteTime)
    Get-Content $err -Tail 10
}
