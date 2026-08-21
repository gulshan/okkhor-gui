# End-to-end test: moving between text fields in one window resets the word.
#
# Two text boxes on one form. The mode is per top-level window, so it stays
# active across the move — but the caret does not, and a word half-typed into
# one field must not continue into the other.
#
# `k` then `x` is the probe. Typed together they are one conjunct, ক্ষ. Typed
# with a reset in between they are ক and then এক্স, because the x is re-read as
# a fresh word. So the second field showing এক্স means the buffer was dropped,
# and showing ক্ষ means it leaked across the move.
#
# Three routes into the same requirement, and only one of them exercises the
# WinEvent focus hook:
#
#   - the application moving focus itself, which is the focus hook alone: no key
#     that classify would treat as a word break, and no click for the mouse hook
#   - Tab, which classify already breaks the word on
#
# The first is the one worth having. Tab would pass even if the focus hook were
# deleted outright, so on its own it would prove nothing about it.
#
#   pwsh -File scripts\e2e-field-switch.ps1

. (Join-Path $PSScriptRoot 'e2e-common.ps1')
Assert-DesktopUnlocked

$KO      = Text-Of 0x0995                                   # ক
$KKHO    = Text-Of 0x0995, 0x09CD, 0x09B7                   # ক্ষ
$EKS     = Text-Of 0x098F, 0x0995, 0x09CD, 0x09B8           # এক্স

# Two fields, so a focus move stays inside one top-level window.
function New-TwoFieldWindow {
    $form = New-Object System.Windows.Forms.Form
    $form.Text = 'okkhor e2e - field switch'
    $form.TopMost = $true
    $form.Width = 460
    $form.Height = 180
    $form.StartPosition = 'Manual'
    $form.Location = New-Object System.Drawing.Point(80, 200)

    $font = New-Object System.Drawing.Font('Nirmala UI', 16)
    $a = New-Object System.Windows.Forms.TextBox
    $a.Font = $font; $a.Width = 420
    $a.Location = New-Object System.Drawing.Point(10, 20)
    $b = New-Object System.Windows.Forms.TextBox
    $b.Font = $font; $b.Width = 420
    $b.Location = New-Object System.Drawing.Point(10, 80)

    $form.Controls.Add($a)
    $form.Controls.Add($b)
    $form.Show()
    Pump 300
    # `Box` is what Use-Window focuses, so the run always starts in the first field.
    return @{ Form = $form; Box = $a; A = $a; B = $b }
}

function Clear-Fields {
    param($Window)
    $Window.A.Clear()
    $Window.B.Clear()
    Pump 250
}

$okkhor = Start-Okkhor
$w = $null

try {
    $w = New-TwoFieldWindow
    Use-Window $w 'first field'

    Tap $VK.F11
    Pump 700

    # Control: with no move in between, the two letters are one conjunct. Without
    # this, every check below would also pass if conversion were broken entirely.
    Type-Keys $VK.K, ([int][char]'X')
    Pump 500
    Check 'kx is one conjunct within a field' $KKHO $w.A.Text

    # The application moves focus itself: no word-breaking key, no click, so the
    # WinEvent focus hook is the only thing that can reset the buffer.
    Clear-Fields $w
    $w.A.Focus() | Out-Null
    Pump 400
    Tap $VK.K
    Pump 400
    Check 'first field holds the k' $KO $w.A.Text

    $w.B.Focus() | Out-Null
    Pump 600
    Tap ([int][char]'X')
    Pump 600
    Check 'programmatic focus move resets the word' $EKS $w.B.Text
    Check 'and the first field is untouched' $KO $w.A.Text

    # Tab, the ordinary way a person moves between fields. classify already
    # treats it as a word break, so this passes through a different path.
    Clear-Fields $w
    $w.A.Focus() | Out-Null
    Pump 400
    Tap $VK.K
    Pump 400
    Tap $VK.Tab
    Pump 600
    Tap ([int][char]'X')
    Pump 600
    Check 'Tab to the next field resets the word' $EKS $w.B.Text
    Check 'and the first field is still untouched' $KO $w.A.Text
}
catch {
    # Foreground lost mid-run. Stop rather than type the rest into whatever took
    # over; Complete-Run reports this as skipped, since nothing was measured.
    if ("$_" -match $ForegroundLost) { $script:Interrupted = $true } else { throw }
}
finally {
    if ($w) { $w.Form.Close(); Pump 200 }
    Stop-Okkhor $okkhor
}

Complete-Run
