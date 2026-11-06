// Messages produced by the Rust backend (src-tauri/src/*.rs). Keys must equal the
// Chinese text exactly as emitted. Keys without placeholders also act as prefixes:
// `前缀: 动态内容` is translated by matching the text before the first colon.
export const backend: Record<string, string> = {
  // shared locks / generic
  数据库锁失败: "Database lock failed",
  运行状态锁失败: "Run state lock failed",
  数据库操作失败: "Database operation failed",

  // runtime.rs: detection details
  "WSH 已被注册表策略禁用 ({key_path})": "WSH is disabled by registry policy ({key_path})",
  "当前进程 PATH: {count} 项": "Current process PATH: {count} entries",
  "用户 PATH: {count} 项": "User PATH: {count} entries",
  "系统 PATH: {count} 项": "System PATH: {count} entries",
  指定的解释器不存在: "The specified interpreter does not exist",
  "#Requires AutoHotkey v2，但没有找到 v2 解释器": "#Requires AutoHotkey v2, but no v2 interpreter was found",
  "未找到 {name} 运行时，请先在“运行时环境”中检测或在系统中配置 PATH":
    "{name} runtime not found. Detect it in Runtimes first, or add it to the system PATH",
  "python.exe 用于控制台脚本，pythonw.exe 仅作为 GUI 候选展示":
    "python.exe is used for console scripts; pythonw.exe is listed only as a GUI candidate",
  "已合并 py -0p 列出的所有 Python 安装": "Merged all Python installations listed by py -0p",
  "未检测到 WSL 发行版；当前 Shell 运行时为 Git Bash": "No WSL distribution detected; the current Shell runtime is Git Bash",
  "检测到 WSL 发行版: {distros}（本版本仍使用 Git Bash 执行 .sh）":
    "WSL distributions detected: {distros} (this version still runs .sh with Git Bash)",
  "Windows 11 中 Windows Script Host 为可选功能；若已移除或策略禁用，脚本无法启动":
    "Windows Script Host is an optional feature in Windows 11; scripts cannot start if it is removed or disabled by policy",
  "未在 PATH、注册表或常见安装目录中找到可执行文件":
    "No executable found in PATH, the registry, or common install directories",
  运行时检测线程异常退出: "The runtime detection thread exited unexpectedly",

  // runner.rs
  "创建 Job Object 失败": "Failed to create Job Object",
  "配置 Job Object 失败": "Failed to configure Job Object",
  打开子进程失败: "Failed to open child process",
  "将子进程加入 Job Object 失败": "Failed to add child process to Job Object",
  "没有进程可以加入 Job Object": "No process available to add to Job Object",
  创建线程快照失败: "Failed to create thread snapshot",
  恢复挂起的子进程线程失败: "Failed to resume the suspended child process thread",
  脚本已禁用: "Script is disabled",
  脚本文件不存在: "Script file does not exist",
  脚本路径不在允许的根目录中: "Script path is not within the allowed root directories",
  工作目录不存在: "Working directory does not exist",
  参数中包含非法空字符: "Arguments contain an illegal null character",
  环境变量名称无效: "Invalid environment variable name",
  "脚本内容哈希不匹配，已拒绝运行": "Script content hash does not match; run refused",
  未知运行时: "Unknown runtime",
  "暂不支持运行 {name}": "Running {name} is not supported yet",
  等待进程结束失败: "Failed to wait for the process to exit",
  运行超时: "Run timed out",
  "进程退出码 {code}": "Process exit code {code}",
  进程因未知原因退出: "The process exited for an unknown reason",
  "成功退出码格式无效": "Invalid success exit codes",
  "{name} 已有 {count} 个实例运行中，已按 skip 策略跳过":
    "{name} already has {count} running instance(s); skipped by the skip policy",
  "{name} 已达到最大实例数 {max}，请调整实例策略":
    "{name} has reached the maximum of {max} instance(s); adjust the instance policy",
  启动进程失败: "Failed to start process",
  "获取子进程 PID 失败": "Failed to get the child process PID",
  创建日志目录失败: "Failed to create log directory",
  创建日志文件失败: "Failed to create log file",
  脚本不存在: "Script does not exist",
  脚本当前没有运行中的实例: "The script has no running instances",

  // scheduler.rs
  未知计划类型: "Unknown schedule type",
  计划关联的脚本不存在: "The script linked to the schedule does not exist",
  计划关联的脚本已禁用: "The script linked to the schedule is disabled",
  等待现有实例结束: "Waiting for the existing instance to finish",
  已错过一次性执行时间: "Missed the one-time run time",
  "达到最大重启次数 {max}，循环已停止": "Reached the maximum of {max} restarts; loop stopped",

  // db.rs
  创建数据目录失败: "Failed to create data directory",
  打开数据库失败: "Failed to open database",
  初始化数据库失败: "Failed to initialize database",
  "升级数据库字段 {column} 失败: {error}": "Failed to upgrade database column {column}: {error}",
  "上次运行退出码 {code}": "Last run exit code {code}",
  上次运行失败: "The last run failed",
  保存脚本失败: "Failed to save script",
  更新脚本失败: "Failed to update script",
  删除脚本失败: "Failed to delete script",
  写入运行记录失败: "Failed to write run record",
  保存计划失败: "Failed to save schedule",
  更新计划失败: "Failed to update schedule",
  保存触发器失败: "Failed to save trigger",
  更新触发器失败: "Failed to update trigger",

  // triggers.rs
  触发器不存在: "Trigger does not exist",
  动作脚本不存在: "The script for the action does not exist",
  未知动作: "Unknown action",
  触发器已触发: "Trigger fired",

  // update.rs
  "连接 GitHub 失败": "Failed to connect to GitHub",
  "GitHub 仓库还没有发布任何版本": "The GitHub repository has no releases yet",
  "GitHub API 访问次数已用完，请稍后再试": "GitHub API rate limit reached. Try again later",
  "检查更新失败：HTTP {status}": "Update check failed: HTTP {status}",
  检查更新失败: "Update check failed",
  解析发布信息失败: "Failed to parse release information",
  初始化网络客户端失败: "Failed to initialize network client",
  "更新地址必须是 HTTPS": "The update URL must use HTTPS",
  "更新地址不在允许的 GitHub 域名内": "The update URL is not on an allowed GitHub domain",
  "下载更新失败：{reasons}": "Failed to download update: {reasons}",
  "更新包体积异常，已停止下载": "Update package size is abnormal; download stopped",
  写入更新包失败: "Failed to write update package",
  下载中断: "Download interrupted",
  下载的更新包为空: "The downloaded update package is empty",
  "下载不完整：{downloaded}/{total} 字节": "Incomplete download: {downloaded}/{total} bytes",
  "下载的文件不是有效的 Windows 安装程序": "The downloaded file is not a valid Windows installer",
  请先检查更新: "Check for updates first",
  当前已是最新版本: "You're on the latest version",
  该版本没有提供安装包: "This version does not provide an installer",
  "该版本没有提供 .sha256 校验文件，已取消安装（无法验证安装包完整性）":
    "This release has no .sha256 checksum file; installation cancelled because the package cannot be verified",
  "更新重定向目标不在允许的 GitHub 域名内":
    "The update redirect target is not on an allowed GitHub domain",
  重定向次数过多: "Too many redirects",
  创建更新目录失败: "Failed to create update directory",
  "更新包校验失败：SHA256 不匹配": "Update package verification failed: SHA256 mismatch",
  "启动安装程序失败：错误码 {code}": "Failed to start installer: error code {code}",
  启动安装程序失败: "Failed to start installer",
  "只允许打开 GitHub 链接": "Only GitHub links can be opened",
  // native notifications (also emitted to the frontend as app-notification)
  发现新版本: "New version available",

  // lib.rs
  读取日志失败: "Failed to read log",
  运行时检测任务失败: "Runtime detection task failed",
  无法根据扩展名识别脚本类型: "Cannot identify the script type from its extension",
  未命名脚本: "Untitled script",
  脚本已在列表中: "Script is already in the list",
  脚本名称不能为空: "Script name is required",
  创建脚本目录失败: "Failed to create script directory",
  写入脚本文件失败: "Failed to write script file",
  删除脚本文件失败: "Failed to delete script file",
  启动任务失败: "Failed to start task",
  停止任务失败: "Failed to stop task",
  接管任务失败: "Failed to adopt processes",
  读取日志任务失败: "Log read task failed",
  打开文件失败: "Failed to open file",
  该运行没有日志文件: "This run has no log file",
  日志文件不存在: "Log file does not exist",
  计划名称不能为空: "Schedule name is required",
  计划不存在: "Schedule does not exist",
  触发器名称不能为空: "Trigger name is required",
  "触发器条件 JSON 无效": "Invalid trigger condition JSON",
  "触发器动作 JSON 无效": "Invalid trigger action JSON",
  不允许的触发器动作: "Trigger action not allowed",
  注册快捷键失败: "Failed to register hotkey",
  打开主窗口失败: "Failed to open the main window",

  // autostart.rs / logging.rs
  打开启动项注册表失败: "Failed to open the startup registry key",
  获取程序路径失败: "Failed to get the program path",
  写入启动项失败: "Failed to write the startup entry",
  读取脚本文件失败: "Failed to read script file",

  // cron.rs
  无法解析时间字段: "Cannot parse time field",
  时间字段超出范围: "Time field out of range",
  "Cron 字段为空": "Cron field is empty",
  无法解析步长: "Cannot parse step",
  "Cron 步长必须大于 0": "Cron step must be greater than 0",
  "Cron 范围起点大于终点": "Cron range start is greater than its end",
  "Cron 表达式需要 5 段（分 时 日 月 周）或 6 段（秒 分 时 日 月 周）":
    "A cron expression needs 5 fields (minute hour day month weekday) or 6 fields (second minute hour day month weekday)",
  "Cron 计算日期溢出": "Cron date calculation overflowed",
  "Cron 表达式不会匹配任何日期（例如 2 月 30 日）":
    "This cron expression never matches any date (e.g. February 30)",
  "未来 8 年内没有匹配的执行时间": "No matching run time within the next 8 years",
  未知时区: "Unknown time zone",
  时间戳无效: "Invalid timestamp",
  间隔不能为空: "Interval cannot be empty",
  无法解析间隔: "Cannot parse interval",
  "间隔必须大于 0": "Interval must be greater than 0",
  "无法解析执行时间，请使用 2026-01-01 12:00 或 RFC3339":
    "Cannot parse the run time. Use 2026-01-01 12:00 or RFC3339",
};
