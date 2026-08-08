# okkhor-gui

Background Bangla phonetic input for Windows. Type Avro-style romanised English
in any application and watch it become Bangla as you type. Transliteration comes
from the [`okkhor`](https://crates.io/crates/okkhor) crate.

```
ami banglay gan gai.   ->   আমি বাংলায় গান গাই।
```

Case matters, because Avro uses it to pick different consonants — `s` is স but
`S` is শ, `t` is ত but `T` is ট. Punctuation okkhor has conversions for is
converted too:

| typed | becomes | |
|-------|---------|-|
| `.` | `।` | danda; stays a `.` in front of a digit, so `3.14` is `৩.১৪` |
| `..` | `।।` | |
| `:` | `ঃ` | visarga |
| `^` | `ঁ` | candrabindu |
| `,,` | `্‌` | hasant + ZWNJ |
| `$` | `৳` | taka sign |

A trailing backtick escapes any of them: `` .` `` types a literal full stop.

## Installing

The executable is its own installer. Double-click it and it offers to install
for the current user; choose No to run it from where it sits instead.

```
okkhor-gui.exe --install      install without asking
okkhor-gui.exe --uninstall    remove it
okkhor-gui.exe --portable     run from here, never ask
--silent                      suppress all dialogs, for scripting
```

Installing copies the program to `%LOCALAPPDATA%\Programs\okkhor-gui`, adds a
Start Menu entry, registers in **Settings → Apps → Installed apps**, and turns
on Start with Windows. There is no administrator prompt at any point: the app
is per-user by nature, since it hooks one interactive session.

Uninstall from Settings → Apps like any other application, or run
`--uninstall`. It removes the program, the shortcut, the autostart entry and
every registry value it ever wrote. One artefact survives: the uninstaller
cannot delete its own running executable, so it hands that last step to a copy
in `%TEMP%`, and that copy is left behind. Deleting a file on reboot needs
administrator rights, which a per-user install never asks for. Only ever one
accumulates — each install and uninstall sweeps away the helpers left by
earlier runs.

## Using it

Run `okkhor-gui.exe`. There is no window — look for the tray icon.

| | |
|---|---|
| **F11** | Toggle transliteration for the focused window |
| **F12** | Quit |

Every window starts **inactive**, and the mode is remembered **per window**, not
per application: two Notepad windows can be in different modes at the same time.
The tray icon shows the focused window's mode — green `অ` for active, grey `A`
for inactive — and its tooltip names the application.

Right-clicking the tray icon offers the mode toggle, the backspace setting
described below, a **Start with Windows** switch, and Exit. Autostart is never
enabled behind your back; it writes
`HKCU\Software\Microsoft\Windows\CurrentVersion\Run` only when you tick it.

## How it types

A `WH_KEYBOARD_LL` hook swallows the romanised keystrokes, buffers the word,
re-converts the whole buffer on every keypress, and rewrites the on-screen text
with `SendInput`. Only the part that actually changed is rewritten, so typing
`ami` sends `আ`, then `ম`, then one backspace and `মি`.

A word ends at space, punctuation, Enter, an arrow key, a mouse click or a
focus change. At that point the buffer is dropped without touching the screen,
so a mistake can never cascade past the word you are typing.

### Backspacing over Bangla

This is the one genuinely fragile part, and it is why there is a setting for it.

Rewriting the preview means backspacing over Bangla that is already on screen,
and applications disagree about what one backspace deletes. Notepad, VS Code and
ordinary Win32 edit controls delete one code point. Word and Chromium-based
editors delete a whole grapheme cluster, so the three code points of `ক্ষ`
disappear in a single press.

The default assumes one code point per backspace. If conjuncts come out mangled
in a particular application, tick **Backspace deletes clusters** in the tray menu
while that application is focused. The choice is remembered per executable in
`HKCU\Software\okkhor-gui\EraseMode`.

## Limitations

- **Elevated applications.** Windows blocks a normal-integrity hook from seeing
  keystrokes headed for an elevated window, so Task Manager and admin terminals
  will not transliterate. Running okkhor-gui elevated fixes that, but the
  autostart entry cannot launch an elevated process without a UAC prompt — use a
  Task Scheduler entry with *Run with highest privileges* instead.
- **F11 is claimed globally.** While okkhor-gui runs, browsers do not get F11 for
  fullscreen. F12 is reserved by the Windows debugger engine; if registering
  either hotkey is refused, the program detects both keys in its keyboard hook
  instead.
- **Antivirus.** A global keyboard hook combined with `SendInput` is
  structurally identical to a keylogger. Unsigned builds may be flagged.
- **Secure desktop.** Hooks are disabled during UAC prompts and Ctrl+Alt+Del.
- **Keyboard layout.** The ASCII keys are mapped assuming a QWERTY-compatible
  layout, which Avro romanisation requires anyway. `ToUnicodeEx` is deliberately
  avoided because it mutates the layout's dead-key state as a side effect.
- **Window handle reuse.** Modes are keyed by `HWND`, and Windows recycles those.
  Entries are dropped on `EVENT_OBJECT_DESTROY`, but a recycled handle
  inheriting a closed window's mode is not impossible.
- Games using raw input, and RDP sessions, will not receive the injected text.

## Building

Needs Rust 1.82 or newer (developed against 1.97).

```bash
cargo build --release
```

The only dependencies are `okkhor` and `windows`.

## Tests

```bash
cargo test
```

The cluster segmentation and the on-screen diff are pure logic and carry the
test suite, including an end-to-end simulation that drives the real parser
through the same buffer-and-diff loop the keyboard hook uses and asserts the
modelled screen matches the parser's output after every keystroke.

The parts that need a live desktop — the keyboard hook, the global hotkeys and
`SendInput` injection — cannot be reached from `cargo test`. Those live in
`scripts/`, and drive a real Win32 edit control with synthetic keystrokes,
reading back what the control actually contains:

```bash
cargo build --release
pwsh -File scripts\e2e-typing.ps1
pwsh -File scripts\e2e-punctuation.ps1
pwsh -File scripts\e2e-per-window.ps1
pwsh -File scripts\e2e-install.ps1
```

They exit 0 on success, 1 on a failed check, 2 if they refuse to run, and 3 if
the desktop is locked — injected keystrokes go nowhere on a lock screen, and
that is a skip rather than a failure.

`e2e-typing.ps1` checks inactive passthrough, live conversion after F11,
backspace unwinding the preview, space committing a word, Shift selecting the
other consonant, and F11 toggling back off. `e2e-punctuation.ps1` checks the
conversions in the table above, including the decimal-point exception and the
backtick escape. `e2e-per-window.ps1` checks that two windows in the *same
process* hold independent modes and that a mode survives switching away and
back. `e2e-install.ps1` installs from a temporary folder, checks everything a
Windows application is expected to register, confirms the *installed* copy
transliterates, then uninstalls through the recorded `UninstallString` — the
same one Settings invokes — and checks that nothing is left. It refuses to run
if okkhor-gui is already installed, so it cannot destroy a real installation.

They print per-check PASS/FAIL and exit non-zero on failure. They take over the
keyboard and the foreground window for about half a minute each, so do not type
while they run. Expected Bangla is written as code points inside the scripts so
results cannot depend on file encoding or console rendering.
