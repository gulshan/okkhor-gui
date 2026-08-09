# End-to-end test: punctuation reaches okkhor.
#
# okkhor converts several punctuation characters to real Bangla — `.` to the
# danda, `:` to visarga, `^` to candrabindu, `$` to the taka sign. If the
# keyboard hook classifies any of them as a word break instead of a word
# character, the conversion is silently lost and the raw ASCII lands in the
# target. Only a live desktop exercises that classification, so it is checked
# here rather than in `cargo test`.
#
#   pwsh -File scripts\e2e-punctuation.ps1

. (Join-Path $PSScriptRoot 'e2e-common.ps1')
Assert-DesktopUnlocked

$AMI          = Text-Of 0x0986, 0x09AE, 0x09BF                     # আমি
$AMI_DANDA    = Text-Of 0x0986, 0x09AE, 0x09BF, 0x0964             # আমি।
$AMI_DANDA2   = Text-Of 0x0986, 0x09AE, 0x09BF, 0x0964, 0x0964     # আমি।।
$VISARGA      = Text-Of 0x0983                                     # ঃ
$CANDRABINDU  = Text-Of 0x0981                                     # ঁ
$TAKA         = Text-Of 0x09F3                                     # ৳
$PI           = Text-Of 0x09E9, 0x002E, 0x09E7, 0x09EA             # ৩.১৪

$okkhor = Start-Okkhor
$window = $null

function Reset-Box {
    param($Window)
    # Space ends the buffered word, so the next check starts clean.
    Tap $VK.Space
    $Window.Box.Clear()
    Pump 300
}

try {
    $window = New-TestWindow 'okkhor e2e - punctuation'

    if (-not (Focus-Window $window.Form)) {
        'ABORT: could not take the foreground; keystrokes would land elsewhere.'
        exit 2
    }
    $window.Box.Focus() | Out-Null
    Pump 400

    Tap $VK.F11
    Pump 700

    # The reported bug: a full stop has to become the danda.
    Type-Keys $VK.A, $VK.M, $VK.I, $VK.Period
    Check 'full stop becomes danda' $AMI_DANDA $window.Box.Text

    # A second dot upgrades it, which only works if both dots stay buffered.
    Tap $VK.Period
    Check 'second dot becomes double danda' $AMI_DANDA2 $window.Box.Text
    Reset-Box $window

    # A trailing backtick escapes the danda back to a literal dot.
    Type-Keys $VK.A, $VK.M, $VK.I, $VK.Period, $VK.Grave
    Check 'backtick escapes the danda' "$AMI." $window.Box.Text
    Reset-Box $window

    Tap-Shifted $VK.Semi
    Check 'shift+semicolon becomes visarga' $VISARGA $window.Box.Text
    Reset-Box $window

    Tap-Shifted $VK.D6
    Check 'shift+6 becomes candrabindu' $CANDRABINDU $window.Box.Text
    Reset-Box $window

    Tap-Shifted $VK.D4
    Check 'shift+4 becomes taka sign' $TAKA $window.Box.Text
    Reset-Box $window

    # A dot in front of a digit stays a dot. The preview shows the danda first
    # and corrects itself once the digit arrives.
    Type-Keys $VK.D3, $VK.Period, $VK.D1, $VK.D4
    Check 'decimal point survives' $PI $window.Box.Text
}
catch {
    # Foreground lost mid-run. Stop rather than type the rest into whatever took
    # over; Complete-Run reports this as skipped, since nothing was measured.
    if ("$_" -match $ForegroundLost) { $script:Interrupted = $true } else { throw }
}
finally {
    if ($window) { $window.Form.Close(); Pump 200 }
    Stop-Okkhor $okkhor
}

Complete-Run
