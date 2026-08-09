# End-to-end test: the core typing loop.
#
# Covers that inactive is the default, that F11 turns on live transliteration,
# that backspace unwinds the preview, that a space commits the word and the next
# one still converts, and that F11 turns it back off.
#
#   pwsh -File scripts\e2e-typing.ps1
#
# Takes over the keyboard and the foreground window for roughly half a minute.

. (Join-Path $PSScriptRoot 'e2e-common.ps1')
Assert-DesktopUnlocked

$AMI     = Text-Of 0x0986, 0x09AE, 0x09BF                                # আমি
$AM      = Text-Of 0x0986, 0x09AE                                        # আম
$AM_GAN  = Text-Of 0x0986, 0x09AE, 0x0020, 0x0997, 0x09BE, 0x09A8        # আম গান

$okkhor = Start-Okkhor
$window = $null

try {
    $window = New-TestWindow 'okkhor e2e - typing'

    if (-not (Focus-Window $window.Form)) {
        'ABORT: could not take the foreground; keystrokes would land elsewhere.'
        exit 2
    }
    $window.Box.Focus() | Out-Null
    Pump 400

    # Every window starts inactive, so these keys must reach the control as-is.
    Type-Keys $VK.A, $VK.M, $VK.I
    Check 'inactive by default' 'ami' $window.Box.Text

    $window.Box.Clear()
    Pump 300

    # F11 activates this window; the same keys now convert as they are typed.
    Tap $VK.F11
    Pump 700
    Type-Keys $VK.A, $VK.M, $VK.I
    Check 'F11 activates, ami converts' $AMI $window.Box.Text

    # Backspace pops the buffered romanised character and rewrites the preview.
    Tap $VK.Back
    Check 'backspace unwinds preview' $AM $window.Box.Text

    # Space ends the word and passes through; the next word converts on its own.
    Tap $VK.Space
    Type-Keys $VK.G, $VK.A, $VK.N
    Check 'space commits, next word converts' $AM_GAN $window.Box.Text

    # Avro is case sensitive and the difference is a different consonant, so
    # Shift must reach the parser rather than ending the word: s is স, S is শ.
    Tap $VK.Space
    $window.Box.Clear()
    Pump 300
    Tap $VK.S
    Check 'lowercase s is dontobo sa' (Text-Of 0x09B8) $window.Box.Text

    Tap $VK.Space
    $window.Box.Clear()
    Pump 300
    Tap-Shifted $VK.S
    Check 'capital S is talobbo sha' (Text-Of 0x09B6) $window.Box.Text

    # Dhaka exercises Shift mid-word: D + h is a single conjunct consonant.
    Tap $VK.Space
    $window.Box.Clear()
    Pump 300
    Tap-Shifted $VK.D
    Type-Keys $VK.H, $VK.A, $VK.K, $VK.A
    Check 'Shift mid-word: Dhaka' (Text-Of 0x09A2, 0x09BE, 0x0995, 0x09BE) $window.Box.Text

    # F11 again returns the window to passthrough.
    Tap $VK.Space
    $window.Box.Clear()
    Pump 300
    Tap $VK.F11
    Pump 700
    $window.Box.Clear()
    Pump 300
    Type-Keys $VK.A, $VK.M
    Check 'F11 deactivates' 'am' $window.Box.Text
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
