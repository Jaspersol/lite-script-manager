export const triggers: Record<string, string> = {
  // 脚本, 删除, 禁用, 启用, 编辑, 关闭, 名称, 保存 live in schedules.ts

  // event types
  "进程启动": "Process started",
  "进程关闭": "Process exited",
  "资源进入阈值": "Resource entered threshold",
  "资源退出阈值": "Resource left threshold",
  "受管脚本启动": "Managed script started",
  "受管脚本结束": "Managed script ended",
  "手动触发": "Manual trigger",
  // action types
  "启动脚本": "Start script",
  "停止脚本": "Stop script",
  "重启脚本": "Restart script",
  "启用触发器": "Enable trigger",
  "禁用触发器": "Disable trigger",
  "系统通知": "System notification",
  // metrics
  "系统 CPU %": "System CPU %",
  "系统内存 %": "System memory %",
  "进程数": "Process count",
  "磁盘剩余 GB": "Disk free GB",
  "匹配进程内存字节": "Matched process memory (bytes)",
  "匹配进程内存 MB": "Matched process memory (MB)",
  "匹配进程 CPU %": "Matched process CPU %",

  "触发器名称不能为空": "Trigger name is required",
  "触发器至少需要一个动作": "A trigger needs at least one action",
  "触发器已保存": "Trigger saved",
  "删除触发器": "Delete trigger",
  "删除触发器「{name}」？": "Delete trigger \"{name}\"?",
  "触发器已删除": "Trigger deleted",
  "已触发：{result}": "Triggered: {result}",
  "{metric} {op} {value}，持续 {seconds}s": "{metric} {op} {value}, for {seconds}s",
  "名称 {name}": "Name {name}",
  "路径含 {path}": "Path contains {path}",
  "父进程 {parent}": "Parent {parent}",
  "任意进程": "Any process",
  "新建触发器": "New trigger",
  "正在加载触发器…": "Loading triggers…",
  "还没有触发器": "No triggers yet",
  "例如：游戏进程启动后执行准备脚本，CPU 持续高于 80% 时通知。":
    "For example: run a prep script when a game process starts, or notify when CPU stays above 80%.",
  "冷却 {seconds}s": "Cooldown {seconds}s",
  "上次 {time}": "Last {time}",
  "立即触发": "Trigger now",
  "编辑触发器": "Edit trigger",
  "最近触发记录": "Recent trigger log",
  "还没有触发记录。": "No trigger events yet.",
  "时间": "Time",
  "事件": "Event",
  "结果": "Result",
  "仅用于在触发器列表和通知中识别。": "Only used to identify the trigger in the list and in notifications.",
  "例如：游戏启动": "e.g. Game started",
  "触发时机：进程启动/退出、CPU/内存等资源阈值，或脚本自身的运行生命周期事件。":
    "When to fire: a process starting or exiting, a resource threshold such as CPU or memory, or a script lifecycle event.",
  "进程匹配": "Process match",
  "脚本匹配": "Script match",
  "监听脚本": "Watched script",
  "留空则匹配所有受管脚本。由本触发器自身启动的运行不会再次触发它。":
    "Leave empty to match every managed script. Runs started by this trigger itself never re-fire it.",
  "任意脚本": "Any script",
  "只配置进程名时不需要查询命令行；路径/命令行匹配会增加一次进程查询。":
    "Matching by process name only does not need a command-line lookup. Matching by path or command line adds one extra process query.",
  "进程名": "Process name",
  "要监听的进程文件名，例如 game.exe。": "File name of the process to watch, e.g. game.exe.",
  "父进程名": "Parent process name",
  "可选。只在指定父进程（如 explorer.exe）拉起的进程匹配时触发，用于排除安装器等临时进程。":
    "Optional. Fires only for processes launched by this parent (e.g. explorer.exe). Useful for excluding installers and other temporary processes.",
  "explorer.exe（可选）": "explorer.exe (optional)",
  "路径包含": "Path contains",
  "可选。进程可执行文件路径包含该字符串时才触发，用于区分同名进程。":
    "Optional. Fires only if the executable path contains this string. Useful for telling same-named processes apart.",
  "D:\\\\Games（可选）": "D:\\\\Games (optional)",
  "命令行包含": "Command line contains",
  "可选。进程命令行包含该字符串时才触发。": "Optional. Fires only if the process command line contains this string.",
  "--server（可选）": "--server (optional)",
  "资源条件": "Resource condition",
  "按采样间隔读取一次资源指标，满足条件并持续指定秒数后才触发动作链。":
    "The resource metric is read once per sampling interval. The action chain fires only after the condition has held for the given number of seconds.",
  "指标": "Metric",
  "参与比较的资源指标，例如 CPU 使用率、内存占用或进程数。":
    "The resource metric to compare, such as CPU usage, memory use, or process count.",
  "比较": "Comparison",
  "指标与阈值之间的比较方式，如 ≥、>、≤、< 或 =。": "How the metric is compared with the threshold: ≥, >, ≤, < or =.",
  "阈值": "Threshold",
  "达到该数值并满足持续秒数后触发动作链。CPU 按百分比填写。":
    "The action chain fires once this value is reached and held for the duration. Enter CPU as a percentage.",
  "持续秒数": "Duration (seconds)",
  "条件需连续满足该秒数才触发，用于过滤瞬时尖峰；填 0 表示条件成立立即触发。":
    "The condition must hold for this many seconds before firing, which filters out short spikes. 0 fires as soon as the condition is met.",
  "滞后值": "Hysteresis",
  "指标回落到阈值减去该值以下时才复位，避免在阈值附近抖动导致反复触发。":
    "The trigger resets only after the metric falls below the threshold minus this value, preventing repeated firing when the metric hovers near the threshold.",
  "冷却秒数": "Cooldown (seconds)",
  "一次触发后在该秒数内不再重复触发，0 表示不限制。":
    "After firing, the trigger will not fire again for this many seconds. 0 means no limit.",
  "动作链": "Action chain",
  "触发后按顺序执行的动作，可组合启动、停止、重启脚本，发送通知或启停其他触发器。":
    "Actions run in order after the trigger fires. You can combine starting, stopping, or restarting scripts, sending notifications, and enabling or disabling other triggers.",
  "+ 添加动作": "+ Add action",
  "选择触发器": "Select trigger",
  "通知标题": "Notification title",
  "通知内容": "Notification message",
  "移除动作": "Remove action",
  "启用此触发器": "Enable this trigger",
  "启用后后台按采样间隔监听该事件；禁用时保留配置但不监听。":
    "When enabled, the event is watched in the background at the sampling interval. When disabled, the configuration is kept but not watched.",
};
