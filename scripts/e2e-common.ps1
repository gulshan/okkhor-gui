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
    uint theirs = GetWindowThreadProcessId(foreground, IntPtr.Zero);
    uint mine = GetCurrentThreadId();
    AttachThreadInput(mine, theirs, true);
    ShowWindow(h, 5);
    BringWindowToTop(h);
    bool ok = SetForegroundWindow(h);
    AttachThreadInput(mine, theirs, false);
    return ok;
  }
}
'@
}

# Virtual-key codes used by the tests.
$VK = @{
    A = 0x41; G = 0x47; I = 0x49; M = 0x4D; N = 0x4E
    Back = 0x08; Space = 0x20; F11 = 0x7A; F12 = 0x7B
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

function Tap {
    param([int] $Vk)
    [OkkhorE2E]::keybd_event([byte]$Vk, 0, 0, [UIntPtr]::Zero)
    Pump 60
    [OkkhorE2E]::keybd_event([byte]$Vk, 0, 2, [UIntPtr]::Zero)
    Pump 260
}

function Type-Keys {
    param([int[]] $Vks)
    foreach ($vk in $Vks) { Tap $vk }
}

function Focus-Window {
    param($Form)
    for ($attempt = 0; $attempt -lt 8; $attempt++) {
        [OkkhorE2E]::Force($Form.Handle) | Out-Null
        Pump 400
        if ([OkkhorE2E]::GetForegroundWindow() -eq $Form.Handle) { return $true }
    }
    return $false
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

function Resolve-OkkhorExe {
    $exe = Join-Path $PSScriptRoot '..\target\release\okkhor-gui.exe'
    if (-not (Test-Path $exe)) {
        throw "okkhor-gui.exe not found. Run 'cargo build --release' first."
    }
    return (Resolve-Path $exe).Path
}

function Start-Okkhor {
    $process = Start-Process -FilePath (Resolve-OkkhorExe) -PassThru
    # Give the hooks, hotkeys and tray icon time to install.
    Start-Sleep -Seconds 2
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
