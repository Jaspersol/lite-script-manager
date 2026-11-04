export const settings: Record<string, string> = {
  // page action / notices
  保存设置: "Save settings",
  "保存中…": "Saving…",
  设置已保存: "Settings saved",
  已是最新版本: "Already up to date",
  "正在加载设置…": "Loading settings…",

  // startup & window
  启动与窗口: "Startup & window",
  "开机启动 LiteScriptManager": "Start LiteScriptManager at login",
  "登录 Windows 后自动启动应用并常驻托盘。": "Starts the app automatically after you sign in to Windows and keeps it in the tray.",
  启动时打开快捷面板: "Open quick panel on startup",
  "应用启动后立即弹出快捷面板；关闭后仍可从托盘或快捷键再次打开。": "Shows the quick panel right after the app starts. When off, you can still open it from the tray or with the hotkey.",
  关闭主窗口时退出应用并停止全部脚本: "Exit the app and stop all scripts when the main window closes",
  "勾选后关闭主窗口即退出应用并停止全部受管脚本；不勾选时应用常驻托盘，脚本继续运行。": "When checked, closing the main window exits the app and stops all managed scripts. When unchecked, the app stays in the tray and scripts keep running.",
  打开面板快捷键: "Open panel hotkey",
  "全局快捷键，用于在任意窗口前台唤出快捷面板。使用 Tauri 快捷键格式，例如 Ctrl+Shift+L 或 Alt+Space；留空表示不注册快捷键。": "Global hotkey that brings up the quick panel over any window. Uses the Tauri shortcut format, e.g. Ctrl+Shift+L or Alt+Space. Leave empty to register no hotkey.",
  "例如 Ctrl+Alt+L；留空禁用": "e.g. Ctrl+Alt+L; leave empty to disable",

  // run & logs
  运行与日志: "Runs & logs",
  "默认停止超时（秒）": "Default stop timeout (seconds)",
  "请求停止脚本后等待的秒数。超时仍未退出则强制结束进程，可填 1-60；单个脚本可在脚本设置里覆盖。": "Seconds to wait after a stop request. If the script has not exited by then, the process is killed. Enter 1-60; each script can override this in its settings.",
  日志保留天数: "Log retention (days)",
  "超过天数的运行日志会被自动删除；填 0 表示不按时间清理，仅受日志总大小上限约束。": "Run logs older than this are deleted automatically. Enter 0 to skip age-based cleanup and rely only on the total size limit.",
  "日志总大小上限（MB）": "Total log size limit (MB)",
  "所有日志文件合计的最大占用，超出后从最旧的日志开始删除；填 0 表示不限制总大小。": "Maximum combined size of all log files. Past this limit, the oldest logs are deleted first. Enter 0 for no size limit.",
  "监控采样间隔（毫秒）": "Monitor sampling interval (ms)",
  "进程与资源事件的采样周期：越小触发越及时，但 CPU 占用越高，可填 250-60000。": "Sampling period for process and resource events. Smaller values trigger sooner but use more CPU. Enter 250-60000.",

  // security
  安全: "Security",
  "应用不会默认以管理员权限启动脚本，也不会安装解释器或修改 PATH。触发器动作只允许启动、停止、重启、通知和启停触发器。": "The app does not start scripts as administrator by default, install interpreters, or modify PATH. Trigger actions are limited to start, stop, restart, notify, and enabling/disabling triggers.",
  运行前校验已固定的脚本内容哈希: "Verify pinned script content hash before running",
  "开启后，对已固定内容哈希的脚本在每次启动前比对哈希；文件被改动则拒绝运行。": "When on, scripts with a pinned content hash are checked before every start. If the file has changed, it will not run.",
  "允许的脚本根目录（每行一个）": "Allowed script root directories (one per line)",
  "限制脚本和计划只能运行这些目录下的文件；留空表示允许本机任意路径。": "Restricts scripts and schedules to files under these directories. Leave empty to allow any path on this machine.",
  留空表示不限制: "Leave empty for no restriction",

  // interface
  界面: "Interface",
  "窗口关闭即销毁 WebView；托盘常驻时没有 WebView 内存。面板和主窗口不要同时长期打开。": "Closing a window destroys its WebView, so the tray-only state uses no WebView memory. Avoid keeping the panel and main window open together for long.",
  语言: "Language",
  "切换界面语言，保存设置后同步到托盘面板和通知。": "Switches the interface language. After you save settings, it also applies to the tray panel and notifications.",

  // about & update
  关于与更新: "About & updates",
  "点击「检查更新」从 GitHub Releases 获取最新版本；下载完成后会以静默方式安装并重启应用。": "Click \"Check for updates\" to get the latest version from GitHub Releases. After the download finishes, it installs silently and restarts the app.",
  "更新来自 GitHub Releases（Jaspersol/LiteScriptManager）": "Updates come from GitHub Releases (Jaspersol/LiteScriptManager)",
  "检查中…": "Checking…",
  检查更新: "Check for updates",
  "启动时自动检查更新（每 6 小时最多一次）": "Check for updates on startup (at most once every 6 hours)",
  "启动后在后台检查 GitHub Releases 是否有新版本，最多每 6 小时一次；只提示，不自动下载。": "Checks GitHub Releases for a new version in the background after startup, at most once every 6 hours. It only notifies you and never downloads automatically.",
  "正在下载更新包…": "Downloading update…",
  "下载完成，正在安装…": "Download complete, installing…",
  "正在下载更新包 {percent}%": "Downloading update {percent}%",
  "安装程序启动后应用会自动退出，并在安装完成后重新打开。": "The app exits once the installer starts and reopens after installation finishes.",
  "发现新版本 v{version}": "New version v{version} available",
  下载并安装: "Download and install",
  "在 GitHub 查看": "View on GitHub",
  "下载失败时可直接从 GitHub Release 页面手动下载安装包。": "If the download fails, you can download the installer manually from the GitHub Release page.",
  "当前已是最新版本 v{version}": "You're on the latest version, v{version}",
};
