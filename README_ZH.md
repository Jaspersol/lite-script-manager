<div align="center">

# LiteScriptManager

### 轻量级 Windows 脚本管理器：14 种运行时、定时任务、事件触发、运行历史，全部驻留在托盘

**添加脚本，设置运行时机，然后让它留在系统托盘。不用手写计划任务，也不用到处开终端窗口。**

[![Platform](https://img.shields.io/badge/platform-Windows%2010%20%7C%2011-lightgrey.svg)](#系统要求)
[![Built with Tauri](https://img.shields.io/badge/built%20with-Tauri%202-orange.svg)](https://tauri.app/)
[![Svelte 5](https://img.shields.io/badge/UI-Svelte%205-ff3e00.svg)](https://svelte.dev/)
[![Rust](https://img.shields.io/badge/core-Rust-b7410e.svg)](https://www.rust-lang.org/)

[English](README.md) | 简体中文

**[安装与构建](#安装与构建) · [快速开始](#快速开始) · [功能特性](#功能特性) · [常见问题](#常见问题)**

</div>

## 为什么选择 LiteScriptManager？

个人自动化脚本往往堆成一批 `.py`、`.ps1`、`.bat`、`.ahk` 文件。每种脚本都有各自的解释器、各自的启动和停止方式、各自的输出位置。通过启动器脚本拉起的后台服务，关掉终端后还会留下孤儿进程。

**LiteScriptManager** 把这些脚本放进同一个桌面应用。它检测本机已有的解释器，用正确的命令行运行脚本，把输出写入每次运行独立的日志文件，并在你停止脚本时清理整棵进程树。

- **14 种运行时，只检测不安装** — Python、JavaScript（Node.js）、Shell（Git Bash）、Perl、PHP、R、Lua、Batch、PowerShell、VBScript、JScript、WSF、AutoHotkey、AutoIt。应用不会安装运行时，不会修改 PATH，也不会写入运行时相关注册表
- **干净的进程控制** — 每次运行在开始执行前就绑定到 Windows Job Object。停止时先发送优雅退出信号，超时后终止整棵进程树
- **定时与触发** — 支持 Cron、固定间隔、一次性、退出后循环四种计划，以及进程启动/退出、资源阈值、其他脚本事件触发
- **托盘优先** — 应用启动时只有系统托盘，没有窗口。快捷面板和主窗口按需创建，关闭时销毁 WebView
- **清晰的历史记录** — 每次运行的退出码、耗时、PID、停止原因、峰值内存、CPU 时间和日志尾部
- **本地单用户** — 脚本以当前用户权限运行。数据保存在本地 SQLite 数据库和日志文件中

## 截图

仓库中暂未收录截图。运行应用（见[快速开始](#快速开始)）即可查看快捷面板和主窗口。

## 安装与构建

LiteScriptManager 目前以源码形式分发。你可以自行构建安装包，或以开发模式运行。

### 系统要求

- **操作系统**：Windows 10 或 11
- **Node.js** 20+ 和 npm
- **Rust** stable，使用 MSVC 工具链
- **Visual Studio Build Tools**，包含"使用 C++ 的桌面开发"工作负载
- **Microsoft Edge WebView2 Runtime**

### 开发模式运行

```bash
npm install
npm run tauri dev
```

### 构建安装包

```bash
npm run tauri build
```

NSIS 安装包（`*-setup.exe`）输出到 `src-tauri/target/release/bundle/nsis/`。安装包按用户安装，不需要管理员权限。

只构建 release 可执行文件、不生成安装包（复制出来即可当作便携版使用）：

```bash
npx tauri build --no-bundle
```

### 检查命令

```bash
npm run check
cargo check --manifest-path src-tauri/Cargo.toml
cargo test --manifest-path src-tauri/Cargo.toml
```

### 发布新版本

应用内更新通过 GitHub API 读取 `Jaspersol/Lite-script-manager` 的最新 Release：

1. 同时修改 `package.json`、`src-tauri/Cargo.toml`、`src-tauri/tauri.conf.json` 中的版本号，保持三处一致
2. 打标签并推送：`git tag v0.2.0 && git push origin v0.2.0`
3. `Release` 工作流会自动构建 NSIS 安装包并上传到 Release；标签与应用版本不一致时工作流会直接失败

用户在 **设置 → 关于与更新** 中点击「检查更新」即可升级。如果在安装包旁额外上传 `<安装包名>.exe.sha256`（格式为 `<哈希>  <文件名>`），下载后还会校验 SHA256。

## 快速开始

1. **启动**：应用启动后只显示系统托盘图标，不打开窗口。
   - 左键点击托盘图标，打开快捷面板。
   - 右键点击，选择 **Show LSM** 打开主窗口，选择 **Exit LSM** 退出应用。
   - 调试时可用 `--show-main` 和 `--show-panel` 直接打开对应窗口。
2. **检查运行时**：在主窗口打开运行时页面。14 种运行时各自显示状态（`ready`、`missing`、`broken` 或 `multiple`）、候选路径和失败原因。你可以为单个脚本指定解释器路径。
3. **添加脚本**：按路径添加已有文件，或新建脚本。可设置参数、工作目录、环境变量、解释器覆盖和进程匹配。
4. **运行与停止**：在快捷面板或主窗口中启动、停止脚本。输出写入每次运行独立的日志文件，日志查看器打开时实时推送。
5. **自动化（可选）**：为脚本添加计划或触发器，然后让应用保持在托盘中运行。

> **注意**：只有 LiteScriptManager 在托盘中运行时，计划和触发器才会生效。要在未登录状态下运行需要 Windows 服务，不在当前范围内。

## 功能特性

### 运行时检测

- 检测是只读的：不安装任何东西，不修改 PATH，不写入运行时相关注册表
- **Python**：启动器（`py -0p`）、虚拟环境、`pythonw.exe` 候选项和注册表安装
- **PowerShell**：5.1 和 7.x。**Shell**：Git Bash，WSL 清单单独显示。**AutoHotkey**：v1 和 v2。**AutoIt**，以及 **WSH** 策略状态
- 用户 PATH 与系统 PATH 分开报告
- 检测缓存按可执行文件路径、大小和修改时间失效
- 版本探测超时为 1.5 秒，不阻塞界面

### 脚本执行

- 命令以 argv 数组构造，不拼接 shell 字符串
- 进程以挂起方式创建，绑定 Job Object 后再恢复运行，子孙进程无法逃逸
- 优雅停止（控制台程序发送 `CTRL_BREAK_EVENT`，GUI 程序发送 `WM_CLOSE`），超过停止超时后终止整棵进程树
- 每个脚本可设置实例策略：`parallel`（并行）、`skip`（跳过）、`queue`（排队）、`replace`（替换），以及最大实例数
- 每个脚本可设置最长运行时间、停止模式、成功退出码，以及可选的 SHA-256 内容固定
- 日志：每次运行独立文件，UTF-8 / GBK 解码，单行上限 16 KB，内存尾部 256 KB，仅在日志查看器打开时向界面批量推送，支持保留天数和总大小上限

### 定时任务

- 5 段和 6 段 Cron、固定间隔、一次性、退出后循环
- 支持 IANA、本地或 UTC 时区，按墙钟时间调度并处理夏令时
- 应用重启后恢复计划，休眠唤醒后补跑，补跑策略为 `none`、`once` 或 `all`
- 重叠策略、超时、成功码，以及循环退避（1 秒、2 秒、5 秒、10 秒、30 秒）和最大重启次数
- 单一调度器睡眠到最近一次任务，计划变更时被唤醒

### 事件触发

- **事件**：进程启动、进程退出、资源进入、资源退出、受管脚本启动、受管脚本退出，以及手动触发
- **进程匹配**：进程名、路径、父进程和命令行。只有规则需要时才查询命令行
- **资源条件**：系统 CPU 和内存、进程数量、磁盘剩余空间、匹配进程的 CPU 和内存，支持持续时间条件、滞后阈值和冷却时间
- **动作**：启动、停止、重启脚本，启用或禁用另一个触发器，发送系统通知，以及动作链
- 只有存在需要采样的触发器时才进行监控采样

### 外部进程接管

主窗口或快捷面板打开时，后台会扫描正在运行的进程，查找尚未在内存中受管的脚本。

- 默认情况下，运行时可执行文件的命令行包含脚本路径时即视为匹配
- 启动器脚本常在拉起后台服务后立即退出。请在脚本编辑器的 **进程匹配** 中填入长期存活进程命令行的子串，例如 `pi-web`
- 匹配到的进程会被绑定到新的 Job Object。如果进程已属于其他 Job，停止脚本时会改为按 PID 终止匹配的进程树
- 接管是尽力而为。既没有脚本路径、也没有匹配规则的进程无法关联到脚本

### 历史与设置

- **运行历史**：按脚本、状态和触发来源筛选；查看退出码、耗时、PID、停止原因、峰值内存、CPU 时间、日志尾部，以及打开完整日志
- **设置**：开机启动、启动时打开面板、关闭即退出、默认停止超时、日志保留天数、日志最大总大小、监控采样间隔、全局快捷键、允许的脚本根目录、哈希校验和语言偏好
- 首次启动引导和系统托盘快捷面板

### 检查更新

- **检查**：读取 GitHub 最新 Release，与当前版本比较，并展示发布说明
- **安装**：仅通过 HTTPS 从 `github.com` / `githubusercontent.com` 下载 NSIS 安装包，校验其为 Windows 可执行文件（若 Release 附带 `.sha256` 文件则额外校验摘要），然后以 `/S /UPDATE /R` 静默运行；应用退出，安装程序替换文件后自动重启应用
- **自动检查**：启动时检查，最多每 6 小时一次，可在「设置 → 关于与更新」中关闭
- 更新安装并重启后，应用以新版本号运行，同一版本不会重复提示

## 数据位置

```text
%APPDATA%\com.litescriptmanager.app\
```

- `litescriptmanager.sqlite3`：脚本、计划、触发器、设置和运行历史
- `logs\<script-id>\<run-id>.log`：每次运行的 stdout 和 stderr
- `logs\app.log`：应用警告、错误和 panic 报告

运行历史只保存日志路径和大小，不保存日志正文。日志文件会在启动、修改设置和运行结束时按保留天数和总大小清理。

## 安全模型

- 脚本以当前用户权限运行。应用不会提权，也不会安装运行时
- 可选的根目录白名单限制允许运行的脚本路径
- 可选的 SHA-256 固定会拒绝内容被改动的脚本
- 触发器动作按固定动作列表校验
- 外部进程触发器只能在进程创建后和退出后观察。要在任意进程启动前或退出前拦截，需要内核驱动或 API Hook，不在本项目范围内

## 已知限制

- 外部进程监控默认使用 1 秒快照差分，生命周期极短的进程可能漏检。低延迟的 ETW/WMI 模式需要管理员配置，因此未包含
- 不采样网络指标
- 界面支持简体中文和英文，可在“设置 → 语言”中切换；托盘面板和系统通知同步生效。部分后台日志文件和少数错误详情仍为中文
- 未配置代码签名和自动更新
- 计划和触发器需要应用持续在托盘中运行

## 常见问题

<details>
<summary><strong>支持哪些运行时？</strong></summary>

Python、JavaScript（Node.js）、Shell（Git Bash）、Perl、PHP、R、Lua、Batch、PowerShell、VBScript、JScript、Windows Script File（WSF）、AutoHotkey 和 AutoIt。不支持 Userscript、UnrealScript、C-System 和 Ruby。

</details>

<details>
<summary><strong>它会帮我安装 Python、Node.js 等解释器吗？</strong></summary>

不会。它只检测已安装的内容。如果某个运行时显示 `missing` 或 `broken`，请自行安装，或手动为脚本指定解释器路径。

</details>

<details>
<summary><strong>我的启动器脚本一启动就退出，应用以为服务停止了，怎么办？</strong></summary>

在脚本编辑器的 **进程匹配** 中填入真实服务命令行的子串。应用随后会跟踪这个长期存活的进程，并能在之后停止它。参见[外部进程接管](#外部进程接管)。

</details>

<details>
<summary><strong>应用关闭或我退出登录后，计划还会运行吗？</strong></summary>

不会。应用必须保持在系统托盘中运行。关闭主窗口只会销毁它的 WebView，托盘进程会继续运行，除非你在设置中启用"关闭即退出"。

</details>

<details>
<summary><strong>需要管理员权限吗？</strong></summary>

不需要。安装包按用户安装，脚本以当前用户权限运行。

</details>

<details>
<summary><strong>触发器能在进程启动之前做出反应吗？</strong></summary>

不能。外部进程只能在创建之后和退出之后被检测到。受管脚本是例外，它们支持精确的启动和停止事件。

</details>

## 技术栈

Tauri 2 · Rust · Svelte 5 · TypeScript · Vite · SQLite (WAL)

项目结构：

```text
src/          Svelte 界面（主窗口和快捷面板）
src-tauri/    Rust 核心：运行时检测、执行器、调度器、触发器、数据库、托盘
```
