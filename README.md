# display-rotate

AI-assisted creation

Rotate a **Windows display** to 0° / 90° / 180° / 270° — either from a **PowerShell script** that Task Scheduler (or any other program) can call, or from a **silent tray app** (single Rust exe) that switches orientation from a right-click menu and toggles landscape ↔ portrait on double-click.

Rotating a screen on Windows *looks* trivial — one `ChangeDisplaySettingsEx` call — but in practice it is not. Setting only `dmDisplayOrientation` fails, the vendor control panel has no interface to script, and after the rotation the absolute-positioning input devices may still be stuck in the old orientation. This project packages the working recipe for all three of those hard constraints, in the two shapes you actually want to consume it from: a scriptable CLI and a tray icon.

> **[English](README.md) | [中文](README.zh-CN.md)**

### This is a small utility I wrote for my own use using DeepSeek Harness. It is suggested to just throw the repository link to the agent for deployment. If you find it useful, please give it a star!

## What it does

| Form | What it is | Use it when |
|---|---|---|
| `scripts/rotate-display.ps1` | PowerShell CLI — one orientation change per invocation, exit code on failure | You want it called from Task Scheduler, a hotkey tool, a launcher, or another program |
| Tray app (`display-rotate.exe`) | Single-crate Rust binary, silent (no window), lives in the notification area | You want to flip orientation by hand, a few times a day |

Both drive the same underlying operation (GDI `ChangeDisplaySettingsEx` and CCD `SetDisplayConfig`), so behaviour is identical; only the front-end differs.

## Why this exists

Windows display rotation has three counter-intuitive hard constraints. They are the entire reason this project exists — everything else is plumbing.

**1. Setting `dmDisplayOrientation` alone fails.** If you only flip that field, `ChangeDisplaySettingsEx` returns `DISP_CHANGE_BADMODE`. You must **also** rewrite `dmPelsWidth` / `dmPelsHeight` to the long/short edges that are consistent with the target orientation, and set `DM_DISPLAYORIENTATION | DM_PELSWIDTH | DM_PELSHEIGHT` in `dmFields` in the same call. This is a measured constraint, not a suggestion — width/height and orientation are validated together.

**2. NVIDIA Control Panel has no interface.** It is a closed GUI (`nvcplui.exe`) with no CLI, COM, or scripting surface. And display orientation was never NVIDIA's business anyway: it goes through the Windows generic display driver model (GDI `ChangeDisplaySettingsEx` / CCD `SetDisplayConfig`), which is the same on every GPU vendor. So "script the NVIDIA panel" is the wrong problem — talk to Windows instead.

**3. After rotating, absolute-positioning input may still map to the old orientation.** Touch, pen, and some injected pointer devices can keep using the previous orientation until the display is re-initialized. Physically, "turn the monitor off and on again" fixes it; the software equivalent is a **real unplug/replug at the CCD layer**:

```
QueryDisplayConfig            → take the full path/mode arrays
  → remove the target path from the config
    (compress the mode array and remap modeInfoIdx accordingly)
  → SetDisplayConfig commit   → that display loses signal
  → wait N seconds            (-DownSeconds)
  → re-attach using the original config
```

> ⚠️ Counter-example, do not use as a reset: `SetDisplayConfig(0, NULL, 0, NULL, SDC_TOPOLOGY_EXTEND | SDC_APPLY)` — the call behind the Win+P "Extend these displays" control, implemented by `C:\Windows\System32\DisplaySwitch.exe` — **returns 0 but does nothing when the topology has not changed**. It is a false success and cannot be used to force a re-init. This is why `-Reset topology` exists only for experimentation, and why the CCD unplug/replug cycle is the default.

## CLI script: `scripts/rotate-display.ps1`

Requires PowerShell on Windows — **`pwsh.exe` (PowerShell 7) recommended**. Windows PowerShell 5.1 also runs the whole script (both verified against the same file).

### Options

| Parameter | Values | Default |
|---|---|---|
| `-Orientation` | `0 \| 90 \| 180 \| 270`; aliases `landscape` (= 0) and `portrait` (= 270) | `0` (landscape) |
| `-Device` | GDI device name | `\\.\DISPLAY5` |
| `-Reset` | `none \| ccdcycle \| topology` | `ccdcycle` |
| `-DownSeconds` | seconds the display stays detached during a CCD cycle | `2` |
| `-ListDevices` | list all active displays (`\\.\DISPLAYn` + current orientation) and exit | — |
| `-WhatIf` | print what would be done, execute nothing | — |

`-Reset` semantics:

- `none` — apply the orientation change only, no re-init. Fastest, but absolute-positioning input may stay in the old orientation.
- `ccdcycle` (**default**) — the real CCD unplug/replug described above; the only reliable software re-init.
- `topology` — the `SDC_TOPOLOGY_EXTEND` call. **Known false-success case** (see above); kept for experiments only.

### Examples

```powershell
# NOTE: always pass -ExecutionPolicy Bypass. On a UNC path (\\wsl.localhost\...) PowerShell
# treats the script as remote and the default RemoteSigned policy refuses to run it.
# The flag is process-scoped; it changes nothing on the system.

# List active displays and their current orientation
pwsh -ExecutionPolicy Bypass -File scripts/rotate-display.ps1 -ListDevices

# Rotate DISPLAY5 to portrait, with the default CCD re-init cycle
pwsh -ExecutionPolicy Bypass -File scripts/rotate-display.ps1 -Orientation portrait

# Explicit orientation on a specific display, no re-init, dry run
pwsh -ExecutionPolicy Bypass -File scripts/rotate-display.ps1 -Orientation 90 -Device '\\.\DISPLAY3' -Reset none -WhatIf

# Full re-init with a longer detach window
pwsh -ExecutionPolicy Bypass -File scripts/rotate-display.ps1 -Orientation 270 -DownSeconds 4
```

## Tray app

A single-crate, single-binary Rust program built against bare `windows-sys`, cross-compiled to a Windows exe. It runs windowless and silent in the notification area.

Right-click menu:

- **四个方向单选项** — four radio items for 0° / 90° / 180° / 270°; the checked item reflects the display's actual current orientation, not the last requested one
- **重新初始化显示(拔插)** — run the CCD unplug/replug cycle (constraint 3) without changing orientation
- **退出** — exit

**Double-click the icon** = toggle landscape ↔ portrait (`landscape` = 0°, `portrait` = 270° — the same aliases the script uses).

### One-shot CLI actions

The same exe also runs non-interactively — no tray, does the job, exits. Handy for scheduled tasks, and for testing without going through the tray:

| Action | What it does |
|---|---|
| `--set <0\|90\|180\|270>` | switch to that orientation |
| `--toggle` | toggle landscape 0° ↔ portrait 270° |
| `--cycle` | run only the CCD unplug/replug re-init (orientation unchanged) |
| `--status` | print the display's actual current orientation |

| Option | Meaning | Default |
|---|---|---|
| `--device <\\.\DISPLAYn>` | target display | `\\.\DISPLAY5` |
| `--down-seconds <n>` | detach window for the CCD cycle (floor 1 s) | `2` |
| `--log-dir <path>` | event log directory | `%LOCALAPPDATA%\display-rotate` |
| `--max-log-mb <n>` | total rolling log budget | `5` |
| `--version`, `--help` / `-h` | version / usage | — |

```powershell
.\display-rotate.exe --status
.\display-rotate.exe --set 270
.\display-rotate.exe --cycle --down-seconds 3
```

Exit codes for one-shot actions: `0` success, `1` action failed, `2` bad arguments.
Effective settings precedence: built-in default < `config.toml` < command line.

> These one-shot actions are a convenience of the exe, **not** part of the frozen parameter contract of the script. For anything scripted or scheduled, prefer `scripts/rotate-display.ps1` — it has documented exit codes, `-WhatIf`, and no console caveat.
>
> **Console caveat**: the exe is linked for the Windows *GUI* subsystem, so it never creates a console of its own. Launched from a terminal (or with output redirected) `--status` / `--help` print normally; double-clicked from Explorer it has nowhere to write, so treat the log file `%LOCALAPPDATA%\display-rotate\display-rotate.log` as the reliable channel there.


## Configuration: `config.toml`

The tray app reads a `config.toml` from **next to the exe**. See `examples/config.example.toml` for the shipped example.

```toml
[display]
device = '\\.\DISPLAY5'   # GDI device name (required in practice; the built-in default is DISPLAY5)
# down_seconds = 2         # detach window for the CCD re-init cycle, seconds
```

> **Use single quotes** for the device name — that is the TOML *literal string*, where backslashes are kept verbatim. Do **not** use double quotes: `"..."` is a TOML *basic string* where `\D` / `\d` may be rejected or misread, and the device path gets mangled.

The CLI script does not read this file; pass it `-Device` instead. Keeping one `\\.\DISPLAYn` per place is deliberate — there is no reliable way to guess which of several monitors you mean.

## Build

```bash
# Windows native exe (zig cross-compile, bare windows-sys, no third-party crates)
bash build-win.sh
# artifact: target/x86_64-pc-windows-gnu/release/display-rotate.exe
```

Prerequisites: `rustup target add x86_64-pc-windows-gnu` + [zig](https://ziglang.org/download/); the `ZIG` variable in `build-win.sh` points to the zig executable.

## Deploy

```bash
cp target/x86_64-pc-windows-gnu/release/display-rotate.exe /mnt/c/AI/display-rotate/display-rotate.exe
cp examples/config.example.toml /mnt/c/AI/display-rotate/config.toml   # then edit `device`
```

- The script needs nothing but `pwsh.exe`; copy `scripts/rotate-display.ps1` anywhere and call it by path.
- For the tray app, keep `config.toml` **next to the exe**.
- **Auto-start (optional)**: run the exe from the Startup folder (`Win+R` → `shell:startup`) or a logon Task Scheduler entry. Because the app is windowless, a scheduled task set to "Hidden" is indistinguishable from the Startup shortcut.

## Known pitfalls (all measured)

- **`EnumDisplaySettings` does not fill in `dmDeviceName`** — what you read back is garbage. To tell a physical display from a virtual one, use the CCD `outputTechnology` field instead: `0xA` = DisplayPort external, `0x80000000` = Internal, `0x10` / `0x11` = IndirectWired / Virtual.
- **`DEVMODE.dmSize` must match the struct you actually marshal**: **156** for `DEVMODEA` (ANSI) and **220** for `DEVMODEW` (Unicode) — the 64-byte gap is `dmDeviceName` + `dmFormName` widening from `CHAR[32]` to `WCHAR[32]`. The PowerShell script uses `DEVMODEA`/156; the Rust tray uses `EnumDisplaySettingsW`/`DEVMODEW`/220. A wrong size does **not** error out — it silently stops `dmPelsWidth`/`dmPelsHeight` from being filled, which are exactly the two fields the rotation constraint depends on.
- **CCD and GDI use two different rotation enums.** CCD `rotation` is `1/2/3/4` = IDENTITY / ROT90 / ROT180 / ROT270, while GDI `dmDisplayOrientation` is `0/1/2/3` = 0° / 90° / 180° / 270°. Never pass a value from one directly into the other.
- **Topology flags cannot be combined with `SDC_SAVE_TO_DATABASE` / `SDC_ALLOW_CHANGES`** — the call returns 87 (`ERROR_INVALID_PARAMETER`).
- 🔴 **Never use `SC_MONITORPOWER`** (`WM_SYSCOMMAND` `0xF170`) as a way to "turn the screen off": it puts the monitor into standby and can **drag the whole machine into sleep**. This was hit in practice; it is forbidden in this project.
- **Calling Windows APIs from WSL**: `pwsh` is a Windows process and **does not understand `/home/...` paths**. Pass the script through a UNC path — `\\wsl.localhost\Ubuntu-26.04\...`.
- **The UNC path needs `-ExecutionPolicy Bypass`**: a `.ps1` on a UNC path counts as remote, so the default `RemoteSigned` policy throws `SecurityError` before the script runs. This is the top reason a copy-pasted example fails under WSL.
- **`Write-Host "..." -f $a,$b` is a bug in PowerShell** — it parses `-f` as `-ForegroundColor` and errors out. Write `Write-Host ("..." -f $a,$b)` instead.

## Limitations (honest boundaries)

- **Windows only.** The whole mechanism is Win32/CCD; there is no Linux or macOS path.
- **The target display must not be exclusively held** by another process (full-screen exclusive apps, some games, remote-desktop sessions). If it is, the mode change fails — there is no way around that from user mode.
- **One display at a time.** With multiple monitors you must name the target explicitly: `-Device '\\.\DISPLAYn'` for the script, `config.toml`'s `[display] device` for the tray app. There is no "rotate all" mode.
- **Device names are not stable across reboots, GPU driver updates, or cable changes.** Run `-ListDevices` again and re-point `config.toml` / `-Device` when a display is not found.
- **This project ships no installer and no auto-update.** Deployment is "copy the exe".

## Related

Same stack as [llama-watch](https://github.com/Milliontimes/llama-watch) and [idm-watch](https://github.com/Milliontimes/idm-watch) (zero third-party deps, zig cross-compile, silent Windows background utility).

## License

MIT — Copyright (c) 2026 Milliontimes. See [LICENSE](LICENSE).
