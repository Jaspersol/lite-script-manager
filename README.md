<div align="center">

# LiteScriptManager

### A Lightweight Script Manager for Windows: 14 Runtimes, Schedules, Triggers, and Run History in the Tray

**Add a script, set when it should run, and let it stay in the system tray. No hand-written Task Scheduler entries, no scattered terminal windows.**

[![Platform](https://img.shields.io/badge/platform-Windows%2010%20%7C%2011-lightgrey.svg)](#system-requirements)
[![Built with Tauri](https://img.shields.io/badge/built%20with-Tauri%202-orange.svg)](https://tauri.app/)
[![Svelte 5](https://img.shields.io/badge/UI-Svelte%205-ff3e00.svg)](https://svelte.dev/)
[![Rust](https://img.shields.io/badge/core-Rust-b7410e.svg)](https://www.rust-lang.org/)

English | [简体中文](README_ZH.md)

**[Installation](#installation--build) · [Quick Start](#quick-start) · [Features](#features) · [FAQ](#faq)**

</div>

## Why LiteScriptManager?

Personal automation usually ends up as a pile of `.py`, `.ps1`, `.bat`, and `.ahk` files. Each needs its own interpreter, its own way to be started and stopped, and its own place for output. Background services started from a launcher script leave orphan processes behind when you close the terminal.

**LiteScriptManager** puts these scripts in one desktop app. It detects the interpreters already on your machine, runs each script with the right command line, captures its output into per-run log files, and stops the whole process tree when you ask it to.

- **14 Runtimes, Detect Only** — Python, JavaScript (Node.js), Shell (Git Bash), Perl, PHP, R, Lua, Batch, PowerShell, VBScript, JScript, WSF, AutoHotkey, and AutoIt. The app never installs a runtime, edits PATH, or writes runtime registry values
- **Clean Process Control** — Every run is bound to a Windows Job Object before it starts executing. Stop sends a graceful signal first, then terminates the whole process tree after a timeout
- **Schedules and Triggers** — Cron, interval, one-time, and loop-after-exit schedules, plus triggers on process start/exit, resource thresholds, and other scripts
- **Tray First** — The app starts in the system tray with no window. The quick panel and main window are created on demand, and their WebViews are destroyed when closed
- **Readable History** — Exit code, duration, PID, stop reason, peak memory, CPU time, and the log tail for every run
- **Local and Single-User** — Scripts run with your own privileges. Data stays in a local SQLite database and log files

## Screenshots

No screenshots are checked in yet. Run the app (see [Quick Start](#quick-start)) to see the quick panel and main window.

## Installation & Build

LiteScriptManager is distributed as source for now. Build the installer yourself, or run it in development mode.

### System Requirements

- **OS**: Windows 10 or 11
- **Node.js** 20+ and npm
- **Rust** stable with the MSVC toolchain
- **Visual Studio Build Tools** with the "Desktop development with C++" workload
- **Microsoft Edge WebView2 Runtime**

### Run in Development

```bash
npm install
npm run tauri dev
```

### Build the Installer

```bash
npm run tauri build
```

The NSIS installer (`*-setup.exe`) is written to `src-tauri/target/release/bundle/nsis/`. It installs per user and does not need administrator rights.

To build only the release executable, without an installer (you can copy it and use it as a portable build):

```bash
npx tauri build --no-bundle
```

### Checks

```bash
npm run check
cargo check --manifest-path src-tauri/Cargo.toml
cargo test --manifest-path src-tauri/Cargo.toml
```

### Publishing a Release

In-app updates read the latest GitHub Release of `Jaspersol/LiteScriptManager` through the GitHub API:

1. Bump the version in `package.json`, `src-tauri/Cargo.toml`, and `src-tauri/tauri.conf.json` so all three match
2. Tag and push: `git tag v0.2.0 && git push origin v0.2.0`
3. The `Release` workflow builds the NSIS installer and uploads it to the release; the workflow fails if the tag does not match the app version

Users are notified by **设置 → 关于与更新** (Settings → About & Updates) → **检查更新** (Check for updates). Uploading `<installer>.exe.sha256` next to the installer adds SHA256 verification (format: `<hex digest>  <file name>`).

## Quick Start

1. **Launch**: The app starts in the system tray without opening a window.
   - Left-click the tray icon to open the quick panel.
   - Right-click and choose **Show LSM** for the main window, or **Exit LSM** to quit.
   - `--show-main` and `--show-panel` open the matching window directly, for debugging.
2. **Check Runtimes**: Open the runtimes page in the main window. Each of the 14 runtimes shows a status (`ready`, `missing`, `broken`, or `multiple`), its candidate paths, and the reason for any failure. You can pin a specific interpreter path per script.
3. **Add a Script**: Add an existing file by path, or create a new one. Set arguments, working directory, environment variables, interpreter override, and process match.
4. **Run and Stop**: Start and stop from the quick panel or the main window. Output is written to a per-run log file and streamed to the log viewer while it is open.
5. **Automate (optional)**: Add a schedule or a trigger to the script, then keep the app running in the tray.

> **Note**: Schedules and triggers only fire while LiteScriptManager is running in the tray. Running while signed out would require a Windows service, which is out of scope.

## Features

### Runtime Detection

- Detection is read-only: it does not install anything, change PATH, or write runtime registry values
- **Python**: launcher (`py -0p`), virtual environments, `pythonw.exe` candidates, and registry installs
- **PowerShell**: 5.1 and 7.x. **Shell**: Git Bash, with WSL inventory shown separately. **AutoHotkey**: v1 and v2. **AutoIt**, plus **WSH** policy status
- User PATH and system PATH are reported separately
- The detection cache is invalidated by executable path, size, and modification time
- Version probes have a 1.5-second timeout and do not block the UI

### Script Execution

- Commands are built as argv arrays, never as concatenated shell strings
- Processes are created suspended, bound to a Job Object, then resumed, so descendants cannot escape
- Graceful stop (`CTRL_BREAK_EVENT` for console programs, `WM_CLOSE` for GUI programs), then process-tree termination after the stop timeout
- Per-script instance policy: `parallel`, `skip`, `queue`, or `replace`, with a maximum instance count
- Per-script maximum runtime, stop mode, success exit codes, and optional SHA-256 content pinning
- Logs: per-run files, UTF-8 or GBK decoding, 16 KB line cap, 256 KB in-memory tail, batched UI streaming only while the log viewer is open, retention days, and a total-size cap

### Schedules

- 5-field and 6-field cron, fixed interval, one-time, and loop-after-exit
- IANA, local, or UTC time zone, with DST-safe wall-clock scheduling
- Restart recovery and sleep/wake catch-up, with `none`, `once`, or `all` catch-up policies
- Overlap policies, timeout, success codes, and loop backoff (1s, 2s, 5s, 10s, 30s) with a maximum restart count
- A single scheduler sleeps until the next due run and wakes when schedules change

### Triggers

- **Events**: process started, process exited, resource enter, resource exit, managed script started, managed script exited, and manual
- **Process matching**: name, path, parent, and command line. The command line is only queried when a rule needs it
- **Resource conditions**: system CPU and memory, process count, disk free space, and matched-process CPU and memory, with duration conditions, hysteresis, and cooldown
- **Actions**: start, stop, or restart a script, enable or disable another trigger, send a system notification, and action chains
- Monitoring samples only while at least one trigger needs it

### External Process Adoption

When the main window or quick panel opens, the backend scans running processes for scripts that are not yet managed in memory.

- A script matches by default when the runtime executable's command line contains the script path
- Launcher scripts often exit after starting a background service. Set **进程匹配** (process match) in the script editor to a substring of the long-lived process command line, for example `pi-web`
- Matched processes are attached to a new Job Object. If a process already belongs to another job, stopping the script falls back to terminating the matched process tree by PID
- Adoption is best effort. A process with no script path or matching pattern cannot be associated with a script

### History and Settings

- **Run history**: filters by script, status, and trigger source; exit code, duration, PID, stop reason, peak memory, CPU time, log tail, and full-log opening
- **Settings**: autostart, open the panel on start, close-to-exit, default stop timeout, log retention days, maximum log size, monitor interval, global hotkey, allowed script roots, hash verification, and language preference
- First-run guide and system-tray quick panel

### Updates

- **Check**: reads the latest GitHub Release, compares it with the running version, and shows the release notes
- **Install**: downloads the NSIS installer over HTTPS (only `github.com` / `githubusercontent.com` hosts are accepted), verifies it is a Windows executable (and the SHA256 digest when the release ships a `.sha256` file), then runs it silently with `/S /UPDATE /R`. The app closes, the installer replaces the files, and the app restarts itself
- **Automatic checks**: once per 6 hours on startup, and can be turned off with 设置 → 关于与更新 (Settings → About & Updates)
- After the app restarts, an installed update reports its own version, so no update is offered again for that release

## Data Location

```text
%APPDATA%\com.litescriptmanager.app\
```

- `litescriptmanager.sqlite3`: scripts, schedules, triggers, settings, and run history
- `logs\<script-id>\<run-id>.log`: per-run stdout and stderr
- `logs\app.log`: application warnings, errors, and panic reports

Run history stores log paths and sizes, not log bodies. Log files are pruned by retention days and total size on startup, on settings changes, and when a run completes.

## Security Model

- Scripts run with the current user's privileges. The app does not elevate or install runtimes
- Optional allowed-root checks restrict which script paths may run
- Optional SHA-256 pinning rejects scripts whose content changed
- Trigger actions are validated against a fixed action list
- External process triggers can only observe processes after creation and after exit. Intercepting arbitrary processes before they start or exit would need a kernel driver or API hook, which is out of scope

## Known Limits

- External process monitoring uses a 1-second snapshot diff by default, so very short-lived processes may be missed. A low-latency ETW/WMI mode is not included because it requires administrator setup
- Network metrics are not sampled
- The UI is currently Chinese. The language preference is stored for future localization
- Code signing and auto-update are not configured
- Schedules and triggers need the app to keep running in the tray

## FAQ

<details>
<summary><strong>Which runtimes are supported?</strong></summary>

Python, JavaScript (Node.js), Shell (Git Bash), Perl, PHP, R, Lua, Batch, PowerShell, VBScript, JScript, Windows Script File (WSF), AutoHotkey, and AutoIt. Userscript, UnrealScript, C-System, and Ruby are not supported.

</details>

<details>
<summary><strong>Does it install Python, Node.js, or other interpreters for me?</strong></summary>

No. It only detects what is already installed. If a runtime shows `missing` or `broken`, install it yourself, or point the script at an interpreter path manually.

</details>

<details>
<summary><strong>My launcher script exits right away, so the app thinks the service stopped. What do I do?</strong></summary>

Set **进程匹配** (process match) in the script editor to a substring of the real service's command line. The app then tracks the long-lived process and can stop it later. See [External Process Adoption](#external-process-adoption).

</details>

<details>
<summary><strong>Do schedules run when the app is closed or I am signed out?</strong></summary>

No. The app must keep running in the system tray. Closing the main window only destroys its WebView, and the tray process continues, unless you enable "close-to-exit" in the settings.

</details>

<details>
<summary><strong>Does it need administrator rights?</strong></summary>

No. The installer is per-user, and scripts run with your current privileges.

</details>

<details>
<summary><strong>Can a trigger react to a process before it starts?</strong></summary>

No. External processes can be detected only after they are created and after they exit. Managed scripts are the exception, and they support precise start and stop events.

</details>

## Tech Stack

Tauri 2 · Rust · Svelte 5 · TypeScript · Vite · SQLite (WAL)

Project layout:

```text
src/          Svelte UI (main window and quick panel)
src-tauri/    Rust core: runtime detection, runner, scheduler, triggers, database, tray
```
