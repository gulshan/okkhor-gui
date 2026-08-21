# Shared helpers for the desktop end-to-end tests.
#
# These cover the parts of okkhor-gui that cannot be reached from `cargo test`:
# the keyboard hook, the global hotkeys and `SendInput` injection. They drive a
# real Win32 edit control with synthetic keystrokes and read back what the
# control actually contains.
#
# Dot-source this file; do not run it directly.

Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing

if (-not ([System.Management.Automation.PSTypeName]'OkkhorE2E').Type) {
    Add-Type @'
using System;
using System.Runtime.InteropServices;
public class OkkhorE2E {
  [DllImport("user32.dll")] public static extern void keybd_event(byte vk, byte scan, uint flags, UIntPtr extra);
  [DllImport("user32.dll")] public static extern uint MapVirtualKeyW(uint code, uint mapType);

  // Inject a key with its real scan code. Passing 0 for the scan code looks
  // like it works for ordinary keys, but Windows will not apply a modifier
  // injected that way to the keystroke that follows it: Shift silently does
  // nothing and every capital arrives lowercase.
  public static void Key(byte vk, bool up) {
    byte scan = (byte)MapVirtualKeyW(vk, 0 /* MAPVK_VK_TO_VSC */);
    keybd_event(vk, scan, up ? 2u : 0u, UIntPtr.Zero);
  }
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
  [DllImport("user32.dll")] public static extern bool BringWindowToTop(IntPtr h);
  [DllImport("user32.dll")] public static extern bool AttachThreadInput(uint a, uint b, bool attach);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, IntPtr pid);
  [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int cmd);
  [DllImport("kernel32.dll")] public static extern uint GetCurrentThreadId();

  // Windows refuses SetForegroundWindow to a process that does not already own
  // the foreground. Attaching to the foreground thread's input queue lifts that
  // restriction for the duration of the call.
  public static bool Force(IntPtr h) {
    IntPtr foreground = GetForegroundWindow();
    uint theirs = foreground == IntPtr.Zero ? 0 : GetWindowThreadProcessId(foreground, IntPtr.Zero);
    uint mine = GetCurrentThreadId();

    // There may be no foreground window at all — right after a process the
    // script launched has exited, for instance — and attaching to our own
    // thread or to a dead one fails. Attach only when it makes sense, and
    // still attempt the raise either way.
    // Deliberately no synthetic Alt tap here. It is the usual trick for
    // regaining foreground privilege, but Alt also activates a window's menu,
    // and DoEvents then pumps inside that modal menu loop and never returns.
    bool attached = theirs != 0 && theirs != mine && AttachThreadInput(mine, theirs, true);

    ShowWindow(h, 5);
    BringWindowToTop(h);
    bool ok = SetForegroundWindow(h);
    if (attached) { AttachThreadInput(mine, theirs, false); }
    return ok;
  }

  [DllImport("user32.dll")] public static extern IntPtr OpenInputDesktop(uint flags, bool inherit, uint access);
  [DllImport("user32.dll")] public static extern bool CloseDesktop(IntPtr h);

  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);

  public static string ForegroundProcess() {
    IntPtr fg = GetForegroundWindow();
    if (fg == IntPtr.Zero) { return ""; }
    uint pid;
    GetWindowThreadProcessId(fg, out pid);
    try { return System.Diagnostics.Process.GetProcessById((int)pid).ProcessName; }
    catch { return ""; }
  }

  // False when nothing can be typed or focused.
  //
  // OpenInputDesktop alone is not enough: a UAC prompt does switch to a secure
  // desktop and makes it fail, but the modern lock screen (LockApp) runs on the
  // ordinary desktop, so the call succeeds while input still goes nowhere. The
  // owner of the foreground window is what actually distinguishes the two.
  public static bool DesktopAvailable() {
    IntPtr desktop = OpenInputDesktop(0, false, 0x0001 /* DESKTOP_READOBJECTS */);
    if (desktop == IntPtr.Zero) { return false; }
    CloseDesktop(desktop);

    string owner = ForegroundProcess();
    return owner != "LockApp" && owner != "LogonUI";
  }

  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetClassNameW(IntPtr h, System.Text.StringBuilder s, int max);
  public static string ForegroundClass() {
    IntPtr fg = GetForegroundWindow();
    if (fg == IntPtr.Zero) { return "(none)"; }
    var sb = new System.Text.StringBuilder(256);
    GetClassNameW(fg, sb, 256);
    return sb.ToString();
  }
}
'@
}

# Virtual-key codes used by the tests. Letters are just their ASCII capitals,
# so a suite needing one that is not here can also pass [int][char]'X'.
$VK = @{
    A = 0x41; D = 0x44; G = 0x47; H = 0x48; I = 0x49; K = 0x4B
    M = 0x4D; N = 0x4E; S = 0x53
    D1 = 0x31; D3 = 0x33; D4 = 0x34; D6 = 0x36
    Back = 0x08; Tab = 0x09; Space = 0x20; LShift = 0xA0; F11 = 0x7A
    # OEM keys, positioned by a US layout — the same assumption the hook makes.
    Period = 0xBE     # .
    Semi   = 0xBA     # ; and, shifted, :
    Grave  = 0xC0     # ` — Avro's "do not combine" marker
}

$Results = @()

# Build a string from code points. Expected Bangla is written this way on
# purpose: it keeps these scripts pure ASCII, so the result can never depend on
# how the file itself was encoded or how the console renders it.
function Text-Of {
    param([int[]] $Codes)
    ($Codes | ForEach-Object { [char]$_ }) -join ''
}

function Codepoints-Of {
    param([string] $Value)
    if ([string]::IsNullOrEmpty($Value)) { return '(empty)' }
    ($Value.ToCharArray() | ForEach-Object { 'U+{0:X4}' -f [int]$_ }) -join ' '
}

# Keep the form's message loop alive while synthetic input is delivered. The
# injected replacement text arrives asynchronously, so every step needs pumping
# rather than a plain sleep.
function Pump {
    param([int] $Milliseconds)
    $deadline = [DateTime]::Now.AddMilliseconds($Milliseconds)
    while ([DateTime]::Now -lt $deadline) {
        [System.Windows.Forms.Application]::DoEvents()
        Start-Sleep -Milliseconds 15
    }
}

# Refuse to inject anything unless the window under test still has the
# foreground.
#
# These tests synthesise real keystrokes, which go wherever the focus is. If you
# switch to another application while they run, every remaining keystroke lands
# in that application instead — and the suites press Backspace and F11, so
# that is not merely a wrong test result but a way to damage whatever you moved
# on to. Aborting is the only safe response; the run reports itself as skipped
# rather than failed, because nothing was actually measured.
$ForegroundLost = 'okkhor-e2e-foreground-lost'

function Assert-Foreground {
    if (-not $script:GuardHwnd) { return }
    if ([OkkhorE2E]::GetForegroundWindow() -eq $script:GuardHwnd) { return }

    # One quiet attempt to take it back, in case something transient stole it.
    [OkkhorE2E]::Force($script:GuardHwnd) | Out-Null
    Pump 300
    if ([OkkhorE2E]::GetForegroundWindow() -eq $script:GuardHwnd) { return }

    throw $script:ForegroundLost
}

# Note the parameter name. PowerShell variable names are case insensitive, so a
# parameter called `$Vk` and the `$VK` table above are the same variable: inside
# such a function `$VK.LShift` silently reads a property off an integer and
# yields $null, which casts to byte 0 and presses nothing. Hence `$KeyCode`,
# here and in Tap-Shifted.
function Tap {
    param([int] $KeyCode)
    Assert-Foreground
    [OkkhorE2E]::Key([byte]$KeyCode, $false)
    Pump 60
    [OkkhorE2E]::Key([byte]$KeyCode, $true)
    Pump 260
}

function Type-Keys {
    param([int[]] $KeyCodes)
    foreach ($code in $KeyCodes) { Tap $code }
}

# Hold Shift across a single tap. The hook reads Shift with GetAsyncKeyState,
# so the modifier has to be genuinely down while the key goes by. Left Shift
# specifically, with its scan code — see the note on OkkhorE2E.Key.
function Tap-Shifted {
    param([int] $KeyCode)
    Assert-Foreground
    [OkkhorE2E]::Key([byte]$VK.LShift, $false)
    Pump 80
    Tap $KeyCode
    [OkkhorE2E]::Key([byte]$VK.LShift, $true)
    Pump 200
}

function Focus-Window {
    param($Form)
    for ($attempt = 0; $attempt -lt 20; $attempt++) {
        [OkkhorE2E]::Force($Form.Handle) | Out-Null
        Pump 400
        if ([OkkhorE2E]::GetForegroundWindow() -eq $Form.Handle) {
            # Everything injected from here on is checked against this window.
            $script:GuardHwnd = $Form.Handle
            return $true
        }
    }
    return $false
}

# Take the foreground for a window built by New-TestWindow and put the caret in
# its text box, ready to be typed into.
#
# Aborting with exit 2 rather than failing is deliberate: without the
# foreground the keystrokes land in another application, so every check would
# compare against an untouched box and report a fault that is not there.
function Use-Window {
    param($Window, [string] $Label)

    if (-not (Focus-Window $Window.Form)) {
        $which = if ($Label) { " ($Label)" } else { '' }
        "ABORT: could not take the foreground$which; keystrokes would land elsewhere."
        exit 2
    }
    $Window.Box.Focus() | Out-Null
    Pump 400
}

function New-TestWindow {
    param([string] $Title, [int] $X = 80, [int] $Y = 200)

    $form = New-Object System.Windows.Forms.Form
    $form.Text = $Title
    $form.TopMost = $true
    $form.Width = 460
    $form.Height = 130
    $form.StartPosition = 'Manual'
    $form.Location = New-Object System.Drawing.Point($X, $Y)

    $box = New-Object System.Windows.Forms.TextBox
    $box.Dock = 'Fill'
    $box.Font = New-Object System.Drawing.Font('Nirmala UI', 16)
    $form.Controls.Add($box)

    $form.Show()
    Pump 200
    return @{ Form = $form; Box = $box }
}

# These tests drive the real desktop, which a locked session does not have.
# Exiting 3 distinguishes "could not run" from a genuine failure.
function Assert-DesktopUnlocked {
    if (-not [OkkhorE2E]::DesktopAvailable()) {
        'SKIP: the session is locked, or a secure desktop (UAC, Ctrl+Alt+Del) is up.'
        'These tests inject keystrokes and take the foreground, so they need an'
        'unlocked, interactive desktop. Sign in and run them again.'
        exit 3
    }
}

function Resolve-OkkhorExe {
    $exe = Join-Path $PSScriptRoot '..\target\release\okkhor.exe'
    if (-not (Test-Path $exe)) {
        throw "okkhor.exe not found. Run 'cargo build --release' first."
    }
    return (Resolve-Path $exe).Path
}

function Start-Okkhor {
    # --portable is required, not cosmetic. Launched with no arguments from
    # outside the install directory the executable acts as its own installer and
    # puts up a modal "install this?" dialog, so the tray app never starts and
    # every check silently sees untransliterated ASCII.
    $process = Start-Process -FilePath (Resolve-OkkhorExe) -ArgumentList '--portable' -PassThru
    # Give the hooks, hotkeys and tray icon time to install.
    Start-Sleep -Seconds 2

    # A second instance quits on the spot, because run_tray_app holds a
    # singleton mutex. Without this check the suite carries on and silently
    # exercises whichever copy already owns that mutex — an installed build, for
    # instance — and every result describes that binary instead of this one.
    # This has happened twice, both times producing a run that looked fine.
    if ($process.HasExited) {
        'ABORT: the build under test exited immediately, so another instance'
        'already holds the singleton mutex. This run would have measured that'
        'copy rather than the one just built. Close or uninstall it, then retry:'
        '    Get-Process okkhor | Format-Table Id, Path'
        exit 2
    }
    return $process
}

function Stop-Okkhor {
    param($Process)
    if ($Process) { Stop-Process -Id $Process.Id -Force -ErrorAction SilentlyContinue }
}

function Check {
    param([string] $Name, [string] $Expected, [string] $Actual)

    $ok = ($Expected -ceq $Actual)
    $script:Results += [pscustomobject]@{ Name = $Name; Ok = $ok }

    if ($ok) {
        "  PASS  $Name -> '$Actual'"
    }
    else {
        "  FAIL  $Name"
        "          expected '$Expected'  [$(Codepoints-Of $Expected)]"
        "          actual   '$Actual'  [$(Codepoints-Of $Actual)]"
    }
}

# Print the tally and set the exit code so these can be chained or scripted.
function Complete-Run {
    ''
    if ($script:Interrupted) {
        'SKIPPED: the foreground moved to another application mid-run.'
        'These tests type real keystrokes, so they stop rather than send the rest'
        'into whatever you switched to. Nothing was measured — run them again and'
        'leave the desktop alone while they work.'
        $global:LASTEXITCODE = 3
        exit 3
    }
    $failed = @($script:Results | Where-Object { -not $_.Ok })
    if ($failed.Count -eq 0) {
        "ALL PASS ($($script:Results.Count) checks)"
        $global:LASTEXITCODE = 0
    }
    else {
        "FAILED $($failed.Count) of $($script:Results.Count) checks"
        $global:LASTEXITCODE = 1
    }
    exit $global:LASTEXITCODE
}
