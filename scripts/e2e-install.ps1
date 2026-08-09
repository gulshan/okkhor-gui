# End-to-end test: the install / uninstall round trip.
#
# Installs from a temporary "downloads" folder exactly as a user would, checks
# everything a standard Windows application is expected to leave behind, proves
# the *installed* copy actually transliterates, then uninstalls through the
# recorded UninstallString — the same string Settings > Apps invokes — and
# checks that every trace is gone.
#
#   pwsh -File scripts\e2e-install.ps1
#
# Refuses to run if okkhor-gui is already installed, so a real installation is
# never destroyed by the test.

. (Join-Path $PSScriptRoot 'e2e-common.ps1')
Assert-DesktopUnlocked

$InstallDir   = Join-Path $env:LOCALAPPDATA 'Programs\okkhor-gui'
$InstalledExe = Join-Path $InstallDir 'okkhor-gui.exe'
$IconFile     = Join-Path $InstallDir 'okkhor-gui.ico'
$Shortcut     = Join-Path $env:APPDATA 'Microsoft\Windows\Start Menu\Programs\okkhor-gui.lnk'
$UninstallKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\okkhor-gui'
$RunKey       = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run'
$SettingsKey  = 'HKCU:\Software\okkhor-gui'

if ((Test-Path $InstallDir) -or (Test-Path $UninstallKey)) {
    'ABORT: okkhor-gui is already installed.'
    'This test installs and then uninstalls, so it will not run against a real'
    'installation. Uninstall first if you want to run it.'
    exit 2
}

$AMI = Text-Of 0x0986, 0x09AE, 0x09BF   # আমি

$staging = Join-Path ([System.IO.Path]::GetTempPath()) "okkhor-gui-e2e-$PID"
$window = $null

# Run a setup command and wait for *that* process only.
#
# Start-Process -Wait waits on the whole process tree, and installing
# deliberately leaves a long-lived tray app running, so -Wait would block until
# the app is killed.
function Invoke-Setup {
    param([string] $Exe, [string[]] $Arguments)
    $process = Start-Process -FilePath $Exe -ArgumentList $Arguments -PassThru
    if (-not $process.WaitForExit(60000)) {
        throw "$Exe $($Arguments -join ' ') did not exit within 60s"
    }
}

function Wait-For {
    param([scriptblock] $Condition, [int] $Seconds = 10)
    $deadline = [DateTime]::Now.AddSeconds($Seconds)
    while ([DateTime]::Now -lt $deadline) {
        if (& $Condition) { return $true }
        Start-Sleep -Milliseconds 250
    }
    return $false
}

try {
    # Stand in for a downloaded copy: install from somewhere that is not the
    # build tree and not the install directory.
    New-Item -ItemType Directory -Path $staging -Force | Out-Null
    $downloaded = Join-Path $staging 'okkhor-gui.exe'
    Copy-Item (Resolve-OkkhorExe) $downloaded

    Invoke-Setup $downloaded @('--install', '--silent')
    Wait-For { Test-Path $InstalledExe } | Out-Null

    Check 'executable installed'   'True' "$(Test-Path $InstalledExe)"
    Check 'icon file written'      'True' "$(Test-Path $IconFile)"
    Check 'start menu shortcut'    'True' "$(Test-Path $Shortcut)"

    $entry = Get-ItemProperty $UninstallKey -ErrorAction SilentlyContinue
    Check 'appears in Apps & Features' 'okkhor-gui' "$($entry.DisplayName)"
    Check 'records a version'          'True'       "$([bool]$entry.DisplayVersion)"
    Check 'records an uninstaller'     "`"$InstalledExe`" --uninstall" "$($entry.UninstallString)"
    Check 'DisplayIcon points at the .ico' $IconFile "$($entry.DisplayIcon)"
    Check 'NoModify is a DWORD'        'Int32'      "$($entry.NoModify.GetType().Name)"

    $run = Get-ItemProperty $RunKey -ErrorAction SilentlyContinue
    Check 'autostart enabled by install' "`"$InstalledExe`"" "$($run.'okkhor-gui')"

    Wait-For { Get-Process okkhor-gui -ErrorAction SilentlyContinue } | Out-Null
    Check 'installed copy is running' 'True' "$([bool](Get-Process okkhor-gui -ErrorAction SilentlyContinue))"

    # Writing a Start Menu shortcut makes the shell announce a newly installed
    # app, and that notification is a foreground-stealing CoreWindow. Let it
    # come and go before trying to claim focus.
    Pump 7000

    # The installed binary must actually work, not merely exist.
    $window = New-TestWindow 'okkhor e2e - installed copy'
    if (Focus-Window $window.Form) {
        $window.Box.Focus() | Out-Null
        Pump 400
        Tap $VK.F11
        Pump 700
        Type-Keys $VK.A, $VK.M, $VK.I
        Check 'installed copy transliterates' $AMI $window.Box.Text
        Tap $VK.F11
        Pump 400
    }
    else {
        Check 'installed copy transliterates' 'focused' `
            "could not take foreground (held by $([OkkhorE2E]::ForegroundClass()))"
    }
    $window.Form.Close()
    $window = $null
    Pump 300

    # Uninstall the way Settings > Apps does: run the recorded string verbatim.
    Invoke-Setup $InstalledExe @('--uninstall', '--silent')
    Wait-For { -not (Test-Path $InstallDir) } -Seconds 20 | Out-Null

    Check 'process stopped'          'False' "$([bool](Get-Process okkhor-gui -ErrorAction SilentlyContinue))"
    Check 'install directory removed' 'False' "$(Test-Path $InstallDir)"
    Check 'shortcut removed'          'False' "$(Test-Path $Shortcut)"
    Check 'Apps & Features entry removed' 'False' "$(Test-Path $UninstallKey)"
    Check 'settings key removed'      'False' "$(Test-Path $SettingsKey)"

    $run = Get-ItemProperty $RunKey -ErrorAction SilentlyContinue
    Check 'autostart entry removed'   'False' "$([bool]$run.'okkhor-gui')"
}
catch {
    # Foreground lost mid-run. Stop rather than type the rest into whatever took
    # over; Complete-Run reports this as skipped, since nothing was measured.
    if ("$_" -match $ForegroundLost) { $script:Interrupted = $true } else { throw }
}
finally {
    if ($window) { $window.Form.Close(); Pump 200 }
    Stop-Process -Name okkhor-gui -Force -ErrorAction SilentlyContinue
    Remove-Item $staging -Recurse -Force -ErrorAction SilentlyContinue
    # Belt and braces: if an assertion failed midway, do not leave the machine
    # with a half-installed app.
    Remove-Item $InstallDir -Recurse -Force -ErrorAction SilentlyContinue
    Remove-Item $Shortcut -Force -ErrorAction SilentlyContinue
    Remove-Item $UninstallKey -Recurse -Force -ErrorAction SilentlyContinue
    Remove-Item $SettingsKey -Recurse -Force -ErrorAction SilentlyContinue
    Remove-ItemProperty -Path $RunKey -Name 'okkhor-gui' -ErrorAction SilentlyContinue
}

Complete-Run
