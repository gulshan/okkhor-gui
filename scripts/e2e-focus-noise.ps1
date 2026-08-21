# End-to-end test: which focus events may drop the word being typed.
#
# Applications announce their internal focus moves with NotifyWinEvent, and
# every one of those reaches an out-of-context WinEvent hook regardless of which
# process it came from. Only one kind means the caret actually moved: a child
# control of the window being typed into taking focus. The app once cleared on
# all of them, so a chat client or Explorer updating itself mid-word silently
# split the word; it then cleared on the target window announcing focus on
# itself, which broke every Chromium address bar.
#
# The three cases are therefore: another process's window (ignore), this
# window's own top level (ignore), a child control of this window (clear).
#
# `kx` is the probe because its meaning depends on an earlier letter: the x only
# becomes ষ when it is read together with the k. Lose the buffer between the two
# and okkhor sees `k` then `x` separately, giving ক followed by এক্স — কএক্স.
# A word like `ami` hides the fault, because আ + মি happens to look right.
#
#   pwsh -File scripts\e2e-focus-noise.ps1

. (Join-Path $PSScriptRoot 'e2e-common.ps1')
Assert-DesktopUnlocked

Add-Type @'
using System;
using System.Runtime.InteropServices;
public class FocusNoise {
  [DllImport("user32.dll")] public static extern void NotifyWinEvent(uint ev, IntPtr hwnd, int idObject, int idChild);
  public const uint EVENT_OBJECT_FOCUS = 0x8005;
  public const int OBJID_CLIENT = -4;
}
'@

$KKHO     = Text-Of 0x0995, 0x09CD, 0x09B7                            # ক্ষ
$KKHOMA   = Text-Of 0x0995, 0x09CD, 0x09B7, 0x09AE, 0x09BE            # ক্ষমা
$AMI      = Text-Of 0x0986, 0x09AE, 0x09BF                            # আমি
$SPLIT    = Text-Of 0x0995, 0x098F, 0x0995, 0x09CD, 0x09B8            # কএক্স

$okkhor = Start-Okkhor
$typing = $null
$other = $null

try {
    $typing = New-TestWindow 'okkhor e2e - focus noise' 80
    $other = New-TestWindow 'unrelated background window' 560

    Use-Window $typing
    Tap $VK.F11
    Pump 700

    # Type $Keys, optionally announcing a focus move from $Source before every
    # keystroke after the first.
    function Type-With-Noise {
        param([string] $Keys, $Source)

        # Re-assert the foreground for every case. Two topmost windows are on
        # screen and the earlier cases end by announcing focus elsewhere, so
        # without this the later cases type into nothing and fail for a reason
        # that has nothing to do with what they are testing.
        if (-not (Focus-Window $typing.Form)) {
            return '(lost foreground)'
        }
        $typing.Box.Focus() | Out-Null
        Pump 300

        $typing.Box.Clear()
        Pump 300

        $first = $true
        foreach ($ch in $Keys.ToCharArray()) {
            if ($Source -and -not $first) {
                [FocusNoise]::NotifyWinEvent(
                    [FocusNoise]::EVENT_OBJECT_FOCUS, $Source,
                    [FocusNoise]::OBJID_CLIENT, 0)
                Pump 150
            }
            $first = $false
            Tap ([int][char]([string]$ch).ToUpper())
        }
        Pump 700
        return $typing.Box.Text
    }

    Check 'kx converts undisturbed' $KKHO (Type-With-Noise 'kx' $null)

    # The real regression: noise from a window that has nothing to do with the
    # one being typed into.
    $background = $other.Form.Handle
    Check 'kx survives background focus'    $KKHO   (Type-With-Noise 'kx' $background)
    Check 'kxoma survives background focus' $KKHOMA (Type-With-Noise 'kxoma' $background)
    Check 'ami survives background focus'   $AMI    (Type-With-Noise 'ami' $background)

    # The other half of the contract, and the distinction is the *level* the
    # event is announced at, not the window it belongs to.
    #
    # A child control taking focus is a real caret move — it may be another
    # field — so the word must be abandoned, and splitting `kx` is what that
    # looks like.
    Check 'child-window focus resets' $SPLIT (Type-With-Noise 'kx' $typing.Box.Handle)

    # The window announcing focus on *itself* is not a caret move, and treating
    # it as one broke every browser address bar: Chromium fires exactly this on
    # its top-level HWND for each keystroke into the omnibox, so `ami` arrived
    # as আমই and `kx` as কএক্স. Verified against Edge 148.
    Check 'own top-level focus does not reset' $KKHO (Type-With-Noise 'kx' $typing.Form.Handle)
}
catch {
    # Foreground lost mid-run. Stop rather than type the rest into whatever took
    # over; Complete-Run reports this as skipped, since nothing was measured.
    if ("$_" -match $ForegroundLost) { $script:Interrupted = $true } else { throw }
}
finally {
    if ($typing) { $typing.Form.Close() }
    if ($other) { $other.Form.Close() }
    Pump 200
    Stop-Okkhor $okkhor
}

Complete-Run
