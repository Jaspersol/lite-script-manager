# LiteScriptManager

A lightweight Windows desktop script manager built with Tauri 2, Svelte 5, and Rust.

## Features

- Manages 14 local runtimes: Python, JavaScript (Node.js), Shell (Git Bash), Perl, PHP, R, Lua, Batch, PowerShell, VBScript, JScript, Windows Script File (WSF), AutoHotkey, and AutoIt.
- Detects installed runtimes and versions without installing, modifying PATH, or writing runtime registry values.
  - Python launcher (`py -0p`), virtual environments, `pythonw.exe` candidates, and registry installs.
  - PowerShell 5.1/7.x, Git Bash, WSL inventory, AutoHotkey v1/v2, AutoIt, and WSH policy status.
  - User PATH and system PATH are reported separately.
  - Detection cache is invalidated by executable path, size, and modification time.
- Adds existing scripts or creates new ones with arguments, working directory, environment, interpreter override, and process matching.
- Runs scripts with argv arrays, a Job Object, suspended-process binding, graceful stop, and process-tree fallback termination.
- Per-script instance policy: parallel, skip, queue, or replace, with a maximum instance count.
- Per-script maximum runtime, stop mode, success exit codes, and optional SHA-256 content pinning.
- Per-run log files with live ring buffer, UTF-8/GBK decoding, 16 KB line cap, 256 KB in-memory tail, batch UI streaming only while the log viewer is open, retention days, and total-size cap.
- Schedules: 5/6-field cron, fixed interval, one-time, and loop-after-exit.
  - IANA/local/UTC timezone support, DST-safe wall-clock scheduling, restart recovery, sleep/wake catch-up, skip/once/all catch-up policies, overlap policies, timeout, success codes, and loop backoff/max restarts.
- Triggers: process started/exited, resource enter/exit, managed script started/exited, and manual.
  - Process name/path/parent/command matching with command-line lookup only when needed.
  - System CPU/memory, process count, disk free, matched-process CPU/memory, duration conditions, hysteresis, cooldown.
  - Actions: start/stop/restart script, enable/disable trigger, system notification, action chains.
- Run history with filters, exit code, duration, PID, stop reason, peak memory, CPU time, log tail, and full-log opening.
- Settings: autostart, panel on start, close-to-exit, default stop timeout, log retention, max log size, monitor interval, global hotkey, allowed script roots, hash verification, and language preference.
- First-run guide and system-tray panel.
- On UI startup, adopts already-running script processes that match the script path or a configured process match.

## External Process Adoption

When the main window or quick panel opens, the backend scans running processes for scripts that are not already managed in memory.

- By default, a script is matched when the runtime executable command line contains the script path.
- Launcher scripts often exit after starting a background service. Set **进程匹配** in the script editor to a substring of the long-lived process command line, for example `pi-web`.
- Matched processes are attached to a new Job Object. If a process cannot be attached because it already belongs to another job, stopping the script falls back to terminating the matched process tree by PID.
- Adoption is best effort. Processes with no script path or matching pattern cannot be associated with a script.

## Requirements

- Windows 10 or 11
- Node.js 20+ and npm
- Rust stable with the MSVC toolchain
- Visual Studio Build Tools with the "Desktop development with C++" workload
- Microsoft Edge WebView2 Runtime

## Run

```bash
npm install
npm run tauri dev
```

The app starts in the system tray without opening a window.

- Left-click the tray icon to open the quick panel.
- Right-click and choose **Show LSM** for the main window, or **Exit LSM** to quit.
- `--show-main` and `--show-panel` open the corresponding window for debugging.

## Build

```bash
npm run tauri build
```

The Windows MSI installer is written under `src-tauri/target/release/bundle/msi/`. A release executable without an installer can be built with:

```bash
npx tauri build --no-bundle
```

Useful checks:

```bash
npm run check
cargo test --manifest-path src-tauri/Cargo.toml
cargo check --manifest-path src-tauri/Cargo.toml
```

## Data Location

```text
%APPDATA%\com.litescriptmanager.app\
```

- `litescriptmanager.sqlite3` — scripts, schedules, triggers, settings, run history
- `logs\<script-id>\<run-id>.log` — per-run stdout/stderr
- `logs\app.log` — application warnings, errors, and panic reports

Run history stores log paths and sizes, not log bodies. Log files are pruned by retention days and total size on startup, settings changes, and run completion.

## Security Model

- Scripts run with the current user's privileges; the app does not elevate or install runtimes.
- Optional allowed-root checks restrict which script paths may run.
- Optional SHA-256 pinning rejects scripts whose content changed.
- Trigger actions are validated against a fixed action list.
- External process triggers can only observe processes after creation and after exit. Pre-start/pre-exit interception of arbitrary external processes would require a kernel driver or API hook and is out of scope.

## Known Limits

- External process monitoring uses a 1-second snapshot diff by default. Very short-lived processes may be missed. ETW/WMI low-latency mode is not included because it requires administrator setup.
- Network metrics are not sampled in the first version.
- The UI is currently Chinese; the language preference is stored for future localization.
- Code signing, auto-update, and NSIS are optional release work and are not configured. The standalone release executable can be copied as a portable build.
- The application must remain running in the tray for schedules and triggers to fire. Running while signed out would require a Windows service and is out of scope.
