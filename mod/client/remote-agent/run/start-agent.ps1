param(
    [string]$Root = '',
    [int]$PollMs = 1000,
    [int]$RequestTimeoutSec = 120,
    [switch]$Force,
    [switch]$Supervised
)

$ErrorActionPreference = 'Continue'

# Design note. This agent must be maintainable from the far side of an SMB share, because that is the
# only channel available. Three mechanisms make that true:
#
#   1. SELF-UPDATE - push a new copy of this script to req\agent-update.ps1 and the running agent
#                    installs it and restarts itself. Fixing the agent no longer needs a human to
#                    touch the machine.
#   2. REQUEST TIMEOUT - every request runs in a child with a hard timeout, killed on expiry. A
#                    blocking request can no longer take the agent down with it.
#   3. SUPERVISOR - the outer instance restarts the inner one if the loop exits, so an unexpected
#                    stop does not need a manual restart.
#
# Root resolution and boot logging happen before anything that can fail, because an agent that dies
# silently is indistinguishable from one that was never started.

function Boot-Log([string]$msg) {
    $line = '[' + (Get-Date -Format 'HH:mm:ss') + '] ' + $msg
    try {
        if ($Root -and (Test-Path $Root)) {
            Add-Content -Path (Join-Path $Root 'agent.log') -Value $line -ErrorAction SilentlyContinue
        } else {
            Add-Content -Path (Join-Path $env:TEMP 'coop-agent-boot.log') -Value $line -ErrorAction SilentlyContinue
        }
    } catch { }
    Write-Host $line
}

if ([string]::IsNullOrWhiteSpace($Root)) {
    $here = $PSScriptRoot
    if (-not $here) { $here = Split-Path -Parent $MyInvocation.MyCommand.Definition }
    if ((Split-Path $here -Leaf) -ieq 'run') { $Root = Split-Path $here -Parent } else { $Root = $here }
}
if (-not (Test-Path $Root)) {
    Boot-Log "FATAL: root does not exist: $Root"
    exit 2
}
$Root = (Resolve-Path $Root).Path

$reqDir  = Join-Path $Root 'req'
$outDir  = Join-Path $Root 'out'
$doneDir = Join-Path $Root 'done'

function Ensure-Dirs {
    foreach ($d in @($script:reqDir, $script:outDir, $script:doneDir)) {
        if (-not (Test-Path $d)) { New-Item -ItemType Directory -Force -Path $d -ErrorAction SilentlyContinue | Out-Null }
    }
}

Ensure-Dirs

$selfPath = $MyInvocation.MyCommand.Definition
if (-not $selfPath -or -not (Test-Path $selfPath)) { $selfPath = Join-Path $Root 'run\start-agent.ps1' }

$log = Join-Path $Root 'agent.log'
function Write-Log([string]$msg) {
    Add-Content -Path $log -Value ('[' + (Get-Date -Format 'HH:mm:ss') + '] ' + $msg) -ErrorAction SilentlyContinue
}

$beatPath = Join-Path $Root 'agent.beat'
# The heartbeat runs on its own timer, not from inside the loop. A stuck request must not be able to
# make the agent look dead as well as behave dead - that combination cost hours here, because every
# diagnosis started from "the agent is not running" when it was in fact running and blocked.
$beatTimer = New-Object System.Timers.Timer
$beatTimer.Interval = 3000
$beatTimer.AutoReset = $true
$beatTimer.add_Elapsed({
    try { Set-Content -Path $beatPath -Value ((Get-Date).ToString('o') + ' pid=' + $PID) -ErrorAction Stop } catch { }
})
$beatTimer.Start()
function Write-Beat {
    try { Set-Content -Path $beatPath -Value ((Get-Date).ToString('o') + ' pid=' + $PID) -ErrorAction Stop } catch { }
}

Boot-Log "boot: root=$Root pid=$PID force=$Force supervised=$Supervised"

# Only the outer instance replaces others; an inner one must not kill its own supervisor.
if ($Force -and -not $Supervised) {
    Get-CimInstance Win32_Process -Filter "Name='powershell.exe'" |
        Where-Object { $_.CommandLine -like '*start-agent.ps1*' -and $_.ProcessId -ne $PID } |
        ForEach-Object {
            Boot-Log "boot: -Force, stopping agent pid $($_.ProcessId)"
            Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue
        }
}

# Installed once, so the agent returns after a logon without anyone remembering to start it.
function Install-Autostart {
    $marker = Join-Path $Root '.autostart-installed'
    if (Test-Path $marker) { return }
    try {
        $startup = [Environment]::GetFolderPath('Startup')
        if (-not $startup) { return }
        $cmd = Join-Path $Root 'run\start-agent.cmd'
        if (-not (Test-Path $cmd)) { return }
        $lnk = Join-Path $startup 'coop-agent.lnk'
        $sh = New-Object -ComObject WScript.Shell
        $s = $sh.CreateShortcut($lnk)
        $s.TargetPath = $cmd
        $s.WorkingDirectory = Split-Path $cmd
        $s.WindowStyle = 7
        $s.Description = 'coop remote agent'
        $s.Save()
        Set-Content -Path $marker -Value (Get-Date -Format o)
        Write-Log "autostart shortcut installed: $lnk"
    } catch {
        Write-Log ('autostart install failed: ' + $_.Exception.Message)
    }
}

# Installs a pushed copy of this script and hands over to it. This is the mechanism that removes the
# need for a human on that machine when the agent itself is broken.
function Test-SelfUpdate {
    $incoming = Join-Path $reqDir 'agent-update.ps1'
    if (-not (Test-Path $incoming)) { return $false }
    try {
        $newHash = (Get-FileHash $incoming -Algorithm SHA256).Hash
        $curHash = ''
        if (Test-Path $selfPath) { $curHash = (Get-FileHash $selfPath -Algorithm SHA256).Hash }
        if ($newHash -eq $curHash) {
            Remove-Item $incoming -Force -ErrorAction SilentlyContinue
            return $false
        }
        Write-Log "self-update: installing $($newHash.Substring(0,12)) (was $($curHash -replace '^(.{12}).*','$1'))"
        Copy-Item $selfPath "$selfPath.old" -Force -ErrorAction SilentlyContinue
        Copy-Item $incoming $selfPath -Force
        Remove-Item $incoming -Force -ErrorAction SilentlyContinue

        $cmd = Join-Path $Root 'run\start-agent.cmd'
        if (Test-Path $cmd) {
            Start-Process -FilePath 'cmd.exe' -ArgumentList '/c', "`"$cmd`"" -WindowStyle Hidden
        } else {
            Start-Process -FilePath 'powershell' `
                -ArgumentList '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', $selfPath `
                -WindowStyle Hidden
        }
        Write-Log 'self-update: spawned replacement, exiting'
        return $true
    } catch {
        Write-Log ('self-update failed: ' + $_.Exception.Message)
        Remove-Item $incoming -Force -ErrorAction SilentlyContinue
        return $false
    }
}

function Invoke-Request([string]$id, [string]$claimed) {
    $outPath  = Join-Path $outDir  ($id + '.out')
    $jsonPath = Join-Path $outDir  ($id + '.json')
    $donePath = Join-Path $outDir  ($id + '.done')
    $body = Get-Content -Path $claimed -Raw -ErrorAction SilentlyContinue
    if (-not $body) { $body = '' }

    $started = Get-Date
    $output = ''
    $code = 0

    if ($body -match '^\s*#SPAWN') {
        # Long-lived work: detach it and return at once. No redirect to the share, which would block.
        $cmd = ($body -split "`r?`n" | Where-Object { $_ -notmatch '^\s*#SPAWN' }) -join "`n"
        $spawnPath = Join-Path $Root ('spawn_' + $id + '.ps1')
        Set-Content -Path $spawnPath -Value $cmd -Encoding UTF8
        $p = Start-Process -FilePath 'powershell' `
            -ArgumentList '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', $spawnPath `
            -WindowStyle Hidden -PassThru
        $output = "spawned pid $($p.Id)"
    }
    else {
        $scriptPath = Join-Path $Root ('run_' + $id + '.ps1')
        # The request body must be written here before the wrapper runs it. A rewrite of this file
        # dropped this line once, and every request then failed in under a second with "not
        # recognized" - which reports exit 0 and looks like success.
        Set-Content -Path $scriptPath -Value $body -Encoding UTF8
        # The script writes its own output; Start-Process is used WITHOUT -RedirectStandardOutput.
        #
        # Redirecting a child's handles made Start-Process block until the child exited, and the
        # WaitForExit timeout never got a chance to run - so a 300 s request wedged the agent for
        # 300 s despite a 120 s timeout, and every later request went unserved. Removing the
        # redirection is what actually makes the timeout effective.
        $localOut = Join-Path $env:TEMP ('coop-agent-' + $id + '.out')
        Remove-Item $localOut, ($localOut + '.err') -ErrorAction SilentlyContinue
        $wrapper = "$scriptPath.wrapper.ps1"
        Set-Content -Path $wrapper -Encoding UTF8 -Value @"
try {
    & '$scriptPath' *>&1 | Out-File -FilePath '$localOut' -Encoding UTF8
} catch {
    ('AGENT WRAPPER EXCEPTION: ' + `$_.Exception.Message) | Out-File -FilePath '$localOut' -Encoding UTF8 -Append
}
"@
        try {
            $p = Start-Process -FilePath 'powershell' `
                -ArgumentList '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', $wrapper `
                -WindowStyle Hidden -PassThru

            if (-not $p.WaitForExit($RequestTimeoutSec * 1000)) {
                try { $p.Kill() } catch { }
                $output = "TIMEOUT after ${RequestTimeoutSec}s - child killed. Put long-lived work behind a #SPAWN request."
                $code = 124
            }
            else {
                $code = $p.ExitCode
                if (Test-Path $localOut) { $output = Get-Content $localOut -Raw -ErrorAction SilentlyContinue }
            }
        }
        catch {
            $output = 'AGENT EXCEPTION: ' + $_.Exception.Message
            $code = 1
        }
        finally {
            # The wrapper reads $scriptPath, so it must outlive the wrapper. Deleting it here was a
            # bug: every request failed with "not recognized as the name of a cmdlet" in under a
            # second, which looks like success and hides that nothing ran.
            Remove-Item $wrapper -Force -ErrorAction SilentlyContinue
            Remove-Item $scriptPath -Force -ErrorAction SilentlyContinue
            Remove-Item $localOut, ($localOut + '.err') -Force -ErrorAction SilentlyContinue
        }
    }

    if ($null -eq $output) { $output = '' }
    Set-Content -Path $outPath -Value $output -Encoding UTF8
    $meta = [ordered]@{
        id       = $id
        exit     = [int]$code
        host     = $env:COMPUTERNAME
        user     = $env:USERNAME
        started  = $started.ToString('o')
        finished = (Get-Date).ToString('o')
        seconds  = [math]::Round(((Get-Date) - $started).TotalSeconds, 2)
        timeout  = $RequestTimeoutSec
    }
    Set-Content -Path $jsonPath -Value ($meta | ConvertTo-Json) -Encoding UTF8
    Set-Content -Path $donePath -Value 'done' -Encoding UTF8
    Move-Item -Path $claimed -Destination (Join-Path $doneDir ($id + '.cmd')) -Force -ErrorAction SilentlyContinue
    Write-Log "done $id exit=$code in $($meta.seconds)s"
}

Install-Autostart
Write-Beat
Write-Log "agent started in $Root (pid $PID, timeout ${RequestTimeoutSec}s)"

while ($true) {
    Write-Beat
    Ensure-Dirs

    if (Test-SelfUpdate) { exit 0 }

    if (Test-Path (Join-Path $reqDir 'agent-stop')) {
        Remove-Item (Join-Path $reqDir 'agent-stop') -Force -ErrorAction SilentlyContinue
        Set-Content -Path (Join-Path $Root '.agent-stopped') -Value (Get-Date -Format o)
        Write-Log 'agent-stop requested, exiting'
        exit 0
    }

    $files = @()
    try { $files = Get-ChildItem -Path $reqDir -Filter '*.cmd' -File -ErrorAction SilentlyContinue } catch { }
    foreach ($f in $files) {
        $id = [System.IO.Path]::GetFileNameWithoutExtension($f.Name)
        $claimed = Join-Path $reqDir ($id + '.cmd.running')
        try { Move-Item -Path $f.FullName -Destination $claimed -Force -ErrorAction Stop } catch { continue }
        Write-Log "run $id"
        try { Invoke-Request -id $id -claimed $claimed }
        catch {
            Write-Log ("request $id raised: " + $_.Exception.Message)
            try { Set-Content -Path (Join-Path $outDir ($id + '.out')) -Value ('AGENT EXCEPTION: ' + $_.Exception.Message) } catch { }
            try { Set-Content -Path (Join-Path $outDir ($id + '.done')) -Value 'done' } catch { }
        }
    }

    Start-Sleep -Milliseconds $PollMs
}
