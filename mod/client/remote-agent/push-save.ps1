# Push the host's saved career to the other PC and verify it byte for byte.
#
# The save is the shared world: both clients must open the same file, or they are playing two
# different careers. The hash check is the point of this script, not a formality - a truncated .sav
# in the save folder is offered by the game's own load menu as a corrupt career.

param(
    [string]$Source = '',
    [string]$Slot   = 'save2'
)

$ErrorActionPreference = 'Stop'

if ([string]::IsNullOrWhiteSpace($Source)) { $Source = Join-Path $env:LOCALAPPDATA "F1Manager24\Saved\SaveGames\$Slot.sav" }
if (-not (Test-Path $Source)) { Write-Output "MISSING source: $Source"; exit 2 }

$destDir = Join-Path $env:LOCALAPPDATA 'F1Manager24\Saved\SaveGames'
New-Item -ItemType Directory -Force -Path $destDir | Out-Null
$dest = Join-Path $destDir ((Split-Path $Source -Leaf))

# The game must not be holding the slot while it is replaced.
$game = Get-Process -Name F1Manager24 -ErrorAction SilentlyContinue
if ($game) {
    Write-Output "WARNING: game is running; a save it has open must not be overwritten"
}

$srcLen  = (Get-Item $Source).Length
$srcHash = (Get-FileHash -Path $Source -Algorithm SHA256).Hash

# Keep whatever was there, because the other PC may have its own career in that slot.
if (Test-Path $dest) {
    $backup = $dest + '.' + (Get-Date -Format 'yyyyMMdd-HHmmss') + '.bak'
    Copy-Item -Path $dest -Destination $backup -Force
    Write-Output "backed up existing slot to $(Split-Path $backup -Leaf)"
}

Copy-Item -Path $Source -Destination $dest -Force

$dstLen  = (Get-Item $dest).Length
$dstHash = (Get-FileHash -Path $dest -Algorithm SHA256).Hash

$magic = ''
try {
    $fs = [System.IO.File]::OpenRead($dest)
    $buf = New-Object byte[] 4
    $fs.Read($buf, 0, 4) | Out-Null
    $fs.Close()
    $magic = [System.Text.Encoding]::ASCII.GetString($buf)
} catch { $magic = 'unreadable' }

Write-Output ("source : {0} bytes  {1}" -f $srcLen, $srcHash)
Write-Output ("dest   : {0} bytes  {1}" -f $dstLen, $dstHash)
Write-Output ("magic  : {0}" -f $magic)
Write-Output ("slot   : {0}" -f (Split-Path $dest -Leaf))

if ($srcLen -ne $dstLen -or $srcHash -ne $dstHash) { Write-Output 'VERIFY: FAIL'; exit 1 }
if ($magic -ne 'GVAS') { Write-Output 'VERIFY: FAIL (not a GVAS save)'; exit 1 }
Write-Output 'VERIFY: OK'
exit 0
