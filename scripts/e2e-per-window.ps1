# End-to-end test: modes are per window, not per application.
#
# Two windows in the same process. Activating one must leave the other alone,
# and the first window's mode has to survive switching away and back. This is
# the behaviour that distinguishes HWND keying from keying on the executable or
# the process id, so it is worth its own test.
#
#   pwsh -File scripts\e2e-per-window.ps1

. (Join-Path $PSScriptRoot 'e2e-common.ps1')
Assert-DesktopUnlocked

$AMI     = Text-Of 0x0986, 0x09AE, 0x09BF                                       # আমি
$AMI_GAN = Text-Of 0x0986, 0x09AE, 0x09BF, 0x0020, 0x0997, 0x09BE, 0x09A8       # আমি গান

$okkhor = Start-Okkhor
$a = $null
$b = $null

try {
    $a = New-TestWindow 'okkhor e2e - window A' 80
    $b = New-TestWindow 'okkhor e2e - window B' 560

    if (-not (Focus-Window $a.Form)) { 'ABORT: no foreground on A'; exit 2 }
    $a.Box.Focus() | Out-Null
    Pump 300

    # Activate window A only.
    Tap $VK.F11
    Pump 700
    Type-Keys $VK.A, $VK.M, $VK.I
    Check 'A converts once activated' $AMI $a.Box.Text

    # Window B belongs to the same process but is a different HWND, so it must
    # still be inactive.
    if (-not (Focus-Window $b.Form)) { 'ABORT: no foreground on B'; exit 2 }
    $b.Box.Focus() | Out-Null
    Pump 500
    Type-Keys $VK.A, $VK.M, $VK.I
    Check 'B stays inactive in the same process' 'ami' $b.Box.Text

    # Returning to A must find its mode intact.
    if (-not (Focus-Window $a.Form)) { 'ABORT: no foreground back on A'; exit 2 }
    $a.Box.Focus() | Out-Null
    Pump 500
    Type-Keys $VK.Space, $VK.G, $VK.A, $VK.N
    Check 'A keeps its mode across the switch' $AMI_GAN $a.Box.Text
}
finally {
    if ($a) { $a.Form.Close() }
    if ($b) { $b.Form.Close() }
    Pump 200
    Stop-Okkhor $okkhor
}

Complete-Run
