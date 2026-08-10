# End-to-end test: an unrelated window's focus events must not drop the word.
#
# Applications announce their internal focus moves with NotifyWinEvent, and
# every one of those reaches an out-of-context WinEvent hook regardless of which
# process it came from. okkhor-gui once cleared its typing buffer on all of
# them, so a chat client or Explorer updating itself mid-word silently split the
# word being typed.
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

    # The other half of the contract. A focus move inside the window being typed
    # into is real — the caret may have gone to another field — so the word must
    # still be abandoned there. Splitting `kx` is exactly what that looks like.
    Check 'own-window focus still resets' $SPLIT (Type-With-Noise 'kx' $typing.Form.Handle)
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
