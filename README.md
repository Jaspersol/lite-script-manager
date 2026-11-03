# LiteScriptManager

A lightweight desktop script manager built with Tauri 2, Svelte 5, and Rust.

## Features

- Manages local scripts for 14 runtimes: Python, JavaScript, Shell, Perl, PHP, R, Lua, Batch, PowerShell, VBScript, JScript, Windows Script File (WSF), AutoHotkey, and AutoIt.
- Detects installed runtimes and their versions without installing or modifying anything.
- Adds existing script files or creates new scripts, with editable arguments, working directory, and enabled state.
- Starts and stops scripts, records run history, and writes per-run log files.
- Uses Windows Job Objects to terminate the full process tree.
- On UI startup, adopts already-running script processes that match the script path or a configured process match, then shows them as running with a stop button.
- Starts from the system tray. The quick panel and main window are created on demand to keep idle resource usage low.

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

Install dependencies:

```bash
npm install
```

Start the desktop app in development mode:

```bash
npm run tauri dev
```

The app starts in the system tray without opening a window.

- Left-click the tray icon to open the quick panel.
- Right-click the tray icon and choose **Show LSM** to open the main window.
- Right-click the tray icon and choose **Exit LSM** to quit.

During development, the backend only works inside the Tauri app. Running `npm run dev` alone serves the frontend in a browser and will not provide Tauri commands.

## Build

Create a release build:

```bash
npm run tauri build
```

The installers and bundled executable are written under `src-tauri/target/release/bundle/`.

Useful checks:

```bash
npm run check
cargo check --manifest-path src-tauri/Cargo.toml
```

## Data Location

On Windows, application data is stored under:

```text
%APPDATA%\com.litescriptmanager.app\
```

This directory contains:

- `litescriptmanager.sqlite3` — scripts, settings, and run history
- `logs\<script-id>\<run-id>.log` — captured stdout and stderr for each run

Deleting this directory resets the application data.

## Project Layout

```text
src/                 Svelte frontend (main window and tray panel)
src-tauri/src/       Rust backend, runtime detection, process runner, and SQLite storage
src-tauri/           Tauri configuration and capabilities
static/              Static frontend assets
```
