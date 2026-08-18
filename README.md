# অক্ষর

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

## Scope

Deliberately small. A hotkey, a mode per window, a tray icon, and one setting —
Start with Windows. No layout options, no custom phonetic rules, no dictionary,
no candidate list or suggestion popup. What you type converts as you type it,
and that is the whole of it.

If you want more than that, use
[**অভ্র** (Avro Keyboard)](https://www.omicronlab.com/avro-keyboard.html). It is
the original of this romanisation, it is far more configurable, and it has the
visible suggestion mode this has no equivalent of — you see candidate spellings
and pick one, rather than getting a single conversion as you type.

## Installing

The executable is its own installer. Double-click it and it offers to install
for the current user; choose No to run it from where it sits instead.

```
okkhor.exe --install      install without asking
okkhor.exe --uninstall    remove it
okkhor.exe --portable     run from here, never ask
--silent                  suppress all dialogs, for scripting
```

Installing copies the program to `%LOCALAPPDATA%\Programs\okkhor`, adds a
Start Menu entry, registers in **Settings → Apps → Installed apps**, and turns
on Start with Windows. There is no administrator prompt at any point: the app
is per-user by nature, since it hooks one interactive session.

Names differ by where they are read. Files and registry keys are `okkhor`,
Apps & Features shows **অক্ষর**, and the Start Menu entry is **Okkhor** —
ASCII there on purpose, since Start Menu search matches what you type and
anyone who wants this app is typing on an English keyboard.

Uninstall from Settings → Apps like any other application, or run
`--uninstall`. It removes the program, the shortcut, the autostart entry and
every registry value it ever wrote. One artefact survives: the uninstaller
cannot delete its own running executable, so it hands that last step to a copy
in `%TEMP%`, and that copy is left behind. Deleting a file on reboot needs
administrator rights, which a per-user install never asks for. Only ever one
accumulates — each install and uninstall sweeps away the helpers left by
earlier runs.

## Using it

Run `okkhor.exe`. There is no window — look for the tray icon.

**F11** toggles transliteration for the focused window. That is the only key
the program claims; quitting is done from the tray menu.

Every window starts **inactive**, and the mode is remembered **per window**, not
per application: two Notepad windows can be in different modes at the same time.
The tray icon shows the focused window's mode — green `অ` for active, grey `A`
for inactive — and its tooltip names the application.

Right-clicking the tray icon offers the mode toggle, a **Start with Windows**
switch, and Exit. Autostart is never enabled behind your back; it writes
`HKCU\Software\Microsoft\Windows\CurrentVersion\Run` only when you tick it.

## How it types

A `WH_KEYBOARD_LL` hook swallows the romanised keystrokes and feeds them to
okkhor's `Editor`, which buffers the word, re-converts it on every keypress and
reports the smallest change to the text on screen. That change is applied with
`SendInput`. Typing `ami` sends `আ`, then `ম`, then `ি` — the `ম` already on
screen is kept and the vowel sign lands on it.

A word ends at a space, Backspace, Enter, an arrow key, a mouse click or a
focus change. The buffer is dropped without touching the screen, so a mistake
can never cascade past the word you are typing. The punctuation in the table
above does *not* end a word: it has to stay buffered for `..` to become `।।`
and for the dot in `3.14` to stay a dot.

### Backspacing over Bangla

Rewriting the preview means erasing Bangla that is already on screen, so the
count has to match what one `VK_BACK` removes in the target. One backspace
erases **one code point** in every application measured: Win32 edit controls,
WinForms `TextBox` and `RichTextBox`, WPF `TextBox`, and Chromium — the last
checked end to end by typing into Edge 148 and reading the field back.

Pressing Backspace yourself is a different thing: it ends the word and reaches
the application unchanged, deleting one code point. Mid-word that is what you
want — backspacing over `আমি` leaves `আম`. Over a conjunct it is visible:
`ক্ষ` becomes `ক্`, not `ক`, because the application deletes a code point and
has no idea the conjunct was three of them. Type on and the next word converts
normally.

## Limitations

- **Elevated applications.** Windows blocks a normal-integrity hook from seeing
  keystrokes headed for an elevated window, so Task Manager and admin terminals
  will not transliterate. Running it elevated fixes that, but the
  autostart entry cannot launch an elevated process without a UAC prompt — use a
  Task Scheduler entry with *Run with highest privileges* instead.
- **F11 is claimed globally.** While the app runs, browsers do not get F11 for
  fullscreen. Only one application at a time can hold a hotkey, so if the
  registration is refused the program detects the key in its keyboard hook
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

The only dependencies are `okkhor`, with its `editor` feature, and `windows`.

`.cargo/config.toml` links the C runtime statically, so every DLL the
executable imports ships with Windows itself — there is no Visual C++
Redistributable to install alongside it, which matters for something that
arrives as a single downloaded file. It costs about 90 KB.

## Tests

```bash
cargo test
```

The live preview belongs to okkhor's `Editor` and is tested there. What is
tested here is the contract between that and the keyboard hook: that the
punctuation the hook routes into the buffer is exactly the punctuation okkhor
converts and survives the editor's printable-ASCII filter, that case picks a
different consonant so Shift may not break a word, and that a later letter can
reinterpret an earlier one so no word may be split between letters.

The parts that need a live desktop — the keyboard hook, the global hotkeys and
`SendInput` injection — cannot be reached from `cargo test`. Those live in
`scripts/`, and drive a real Win32 edit control with synthetic keystrokes,
reading back what the control actually contains:

```bash
cargo build --release
pwsh -File scripts\e2e-typing.ps1
pwsh -File scripts\e2e-punctuation.ps1
pwsh -File scripts\e2e-per-window.ps1
pwsh -File scripts\e2e-focus-noise.ps1
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
back. `e2e-focus-noise.ps1` fires focus events from an unrelated background
window mid-word and checks the buffered word survives — `kx` is the probe,
since it only becomes ক্ষ when the x is read together with the k.
`e2e-install.ps1` installs from a temporary folder, checks everything a
Windows application is expected to register, confirms the *installed* copy
transliterates, then uninstalls through the recorded `UninstallString` — the
same one Settings invokes — and checks that nothing is left. It refuses to run
if the app is already installed, so it cannot destroy a real installation.

They print per-check PASS/FAIL and exit non-zero on failure. They take over the
keyboard and the foreground window for about half a minute each, so do not type
while they run. Expected Bangla is written as code points inside the scripts so
results cannot depend on file encoding or console rendering.
