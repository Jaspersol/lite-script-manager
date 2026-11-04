# Bug 排查报告

范围：主页面无法打开、脚本运行状态扫描异常、点击「打开」或运行脚本后托盘面板无法关闭，以及排查中发现的其他问题。

方法：静态阅读 `src-tauri/src/{lib,windows,runner,db,runtime}.rs`、前端 `src/App.svelte`、`src/Panel.svelte`、`src/lib/*`，对照 Tauri 2.12.5 源码文档，并读取本机数据目录 `%APPDATA%\com.litescriptmanager.app` 下的 SQLite 与 `logs/app.log`。**没有运行 GUI 复现**，下面标注 `[推断]` 的结论未经运行验证。

---

## 1. 主页面无法打开，且「打开」按钮后面板关不掉（同一根因）

**严重度：高**

**位置**
- `src-tauri/src/lib.rs:774` `show_main` 是同步命令 (`fn`，不是 `async fn`)
- `src-tauri/src/windows.rs:150` `ensure_main` 在其中调用 `WebviewWindowBuilder::build()`

**原因**
Tauri 的同步命令在主线程（事件循环线程）执行。`WebviewWindowBuilder::build()` 的文档（`tauri-2.12.5/src/webview/webview_window.rs:58,115`）明确写道：

> On Windows, this function deadlocks when used in a synchronous command and event handlers ... You should use `async` commands and separate threads when creating windows.

后果链：
1. 面板里点「打开管理添加 →」→ `api.showMain()` → 同步命令在主线程里创建主窗口 → WebView2 创建需要泵消息，而主线程正在阻塞等待 → 死锁。
2. `ensure_main` 里 `hide_panel(app)` 在 `build()` 之后，永远执行不到，面板不会隐藏。
3. 主线程被卡死后，托盘点击、面板的关闭按钮 (`close_panel`，同样是同步命令)、Esc、窗口消息全部无响应。用户看到的是「面板关不掉」。
4. 数据库佐证：多条 run 记录的错误是「应用上次退出时运行状态未正常结束」（`db.rs:143` 在启动时批量改写），说明应用多次是被强杀而不是正常退出。

**同类调用点（都在事件处理器里创建窗口，受同一条文档约束）** `[推断]`
- `windows.rs:40` 托盘菜单「Show LSM」→ `ensure_main`
- `windows.rs:33` 托盘左键 → `toggle_panel` → `ensure_panel` → `build()`
- `lib.rs:~808` 全局快捷键回调 → `ensure_panel`
- `lib.rs:~802` single-instance 回调 → `ensure_main` / `ensure_panel`

这几处在启动阶段的 `setup` 钩子里调用是安全的（`lib.rs` 的 `--show-main` / `--show-panel` 路径），所以「启动参数能打开、运行中点击打不开」这一现象与此一致。

**建议修复**
- `show_main` 改为 `async fn`（或 `#[tauri::command(async)]`）。
- 事件处理器里不要直接 `build()`，改为 `tauri::async_runtime::spawn` / `std::thread::spawn` 后在该线程里调用 `ensure_main` / `ensure_panel`。
- `ensure_main` 里把 `hide_panel` 放到 `build()` 成功之后的同一路径，并在 `show_main` 里先 `close_panel`，避免依赖窗口创建成功。
- `close_panel` 也应改为 `async`，避免被主线程阻塞牵连。

---

## 2. 面板在没有拿到焦点时永远不会自动关闭

**严重度：中**
**位置**：`src-tauri/src/windows.rs:115-148` `start_dismiss_watchdog`

**原因**
- 只有 `had_focus == true` 之后失去焦点才会 `destroy()`。如果 Windows 拒绝前台请求，面板从未获得焦点，`had_focus` 一直是 `false`，看门狗只重试 6 次 `set_focus()` 就不再做任何事，之后既没有超时，也没有「无焦点则关闭」的兜底。
- 窗口属性是 `always_on_top + skip_taskbar + decorations(false)`，没有任务栏图标，没有任何其他途径关闭（除按钮/Esc，而它们依赖 WebView 获得键盘焦点和主线程不卡死）。
- 运行脚本时由脚本启动器（`cscript` / `AutoHotkey` 等）抢占前台，会让这个状态更容易出现 `[推断]`。

**另一处缺陷：看门狗泄漏**
面板被 `close_panel` 或 Esc 销毁后，如果在 ≤120ms 内用同一 label 重新创建，旧看门狗的 `get_webview_window(PANEL_LABEL)` 返回的是新窗口，所以不会退出；但它持有的是已销毁窗口的句柄，`is_visible()` 返回 `Err` → `unwrap_or(false)` → 一直 `continue`，直到下一次面板被销毁。每次快速重开泄漏一个轮询任务。

**建议修复**
- 看门狗与创建时的窗口一一对应：比较窗口的 webview 标识/用一个代次计数，旧代次直接退出。
- 增加兜底：窗口可见但在 N 秒内始终未获得焦点时，直接关闭或重新 `set_focus` + 置顶。

---

## 3. 脚本运行状态扫描无法得到结果

**严重度：高（对本机实际数据）**

本机三个脚本都是 VBScript（`start_hide_pi`、`start_hide_omp`、`start-hidden`），典型用法是用 `WScript.Shell.Run` 隐藏地启动另一个常驻进程后自己退出。

### 3.1 按「脚本路径」匹配进程，对这类脚本永远匹配不到
- `runner.rs:501-545` `matching_pids`：`process_match` 为空时，只匹配「可执行文件名在运行时白名单内 (`cscript.exe`) 且命令行包含脚本路径」的进程。
- `.vbs` 本身只存活很短时间，真正常驻的是它启动的子进程，其命令行里没有脚本路径，因此启动后重新打开应用时 `adopt_external`（`runner.rs:660`）扫不到任何进程，状态显示为「已停止」。
- 在本次运行内，状态靠 Job Object 保持，所以现象是「刚运行时显示运行中，重启应用后丢失」。
- 数据库里 `process_match` 全部为 `NULL`。

**建议**：UI 上提示这类脚本需要填写「进程匹配」；或者扫描时也匹配父进程链，或记录已启动的子进程路径。

### 3.2 状态只来源于 DB 的最近一条 run 记录，不是实时进程状态
- `db.rs:178-229`：`Script.status` 由 `LEFT JOIN runs ... ORDER BY started_at DESC LIMIT 1` 得到。只要 `finish_run` 没有执行（强杀、死锁、崩溃），状态就永久停在 `running`，直到下一次启动应用时 `db.rs:143` 把它们批量改成 `failed`。
- 即问题 1 的死锁会直接污染这里的状态。

### 3.3 只在窗口 `onMount` 时扫描一次，没有周期刷新
- `App.svelte:326` 和 `Panel.svelte:79` 都只在挂载时调用一次 `adoptExternalRuns`。
- 面板每次打开都是新建窗口，所以会重扫；主窗口常驻后，外部启动/退出的进程不会再被发现。
- 设置里有 `monitor_interval_ms = 1000`，但 `adopt_external` 并不使用它 `[推断，已核对 grep 范围内无引用]`。

### 3.4 `adopt_external` 与 `attach_running` 的已管理 PID 集合不一致
- `runner.rs:687` 使用 `run.pid`（只含主 PID）；`runner.rs:560-568` 使用 `run.pids`（全部 PID）。
- 被接管的 run 含多个 PID 时，其余 PID 在 `adopt_external` 中不会被视为已管理，可能被另一个脚本的 `process_match` 再次接管。

### 3.5 `.pyw` 无法被接管
- `runtime.rs:25` Python 的白名单只有 `python.exe / python3.exe / python / py.exe / py`，没有 `pythonw.exe`。`.pyw` 通常由 `pythonw.exe` 运行，`executable_allowed` 会过滤掉它。

### 3.6 接管后的进程会在应用退出时被杀
- `runner.rs:51` 所有 Job Object 都带 `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`，`attach_many` 对「外部启动的进程」也用同一个 Job。应用退出（或被强杀）时句柄关闭，被接管的外部进程会被一并终止。
- 数据库里 09:44:30 两条 `adopted` 记录紧接着是「应用上次退出时运行状态未正常结束」，与此一致 `[推断]`。
- 对「接管」语义来说这可能不是用户预期；至少应在 UI 说明，或对 adopted 的 Job 不设 kill-on-close。

### 3.7 `start_with` 先尝试接管再启动
- `runner.rs:1369-1373`：用户点「运行」时如果 `attach_running` 命中，直接返回 `running` 事件而不启动新进程。`process_match` 写得宽（例如 `python`、`node`）时，用户点运行永远不会真的启动脚本，也没有任何提示。

---

## 4. 其他问题

| # | 位置 | 问题 | 严重度 |
|---|------|------|--------|
| 4.1 | `scheduler.rs:~382`；`app.log` | 计划 `loop-smoke` 在 skip 策略下，上一实例仍在运行时每秒失败重试，日志里连续出现 158 次「循环 … 第 N 次失败」+「计划 … 启动失败」。重试没有上限/退避生效，会刷满应用日志（`app.log` 已 37KB 且被轮转） | 中 |
| 4.2 | `lib.rs:728` | `apply_hotkey` 标了 `#[tauri::command]`，但参数是 `&AppHandle`，也没有注册到 `invoke_handler`，是无效的多余属性 | 低 |
| 4.3 | `lib.rs:746-767` `update_settings` | 先写 DB，再依次 `autostart::apply(...)?`、`apply_hotkey(...)?`。任一步失败就提前返回，后续的 `scheduler::wake`、`triggers::wake`、`settings-changed` 事件都不会发出，而设置已经落库，前后不一致 | 中 |
| 4.4 | `lib.rs` setup + `autostart.rs:11` | `start_with_windows = true` 时，每次启动都会把 `current_exe()` 写入 `HKCU\...\Run`。开发时运行 `target\debug\litescriptmanager.exe` 会把开机自启指向调试产物（本机该设置为 `true`） | 中 |
| 4.5 | `db.rs:143` | 启动时把所有未结束的 run 统一标成 `failed`，随后 `adopt_external` 又为同一批进程新建一条 `adopted` run。每次重启都会在历史里多出一批「失败」+「接管」记录，难以区分真失败 | 低 |
| 4.6 | `windows.rs:213-232` `toggle_panel` | 已存在但被 `hide_panel` 隐藏的面板，重新显示时不会调用 `panel_position`，位置沿用旧值；分辨率/任务栏变化后可能错位 | 低 |
| 4.7 | `Cargo.toml` `[profile.release]` | `panic = "abort"`：`adopt_external` 内部 panic 会直接终止进程，且 `adopting` 标志不会被复位（本来的 `swap` 保护也就没有意义） | 低 |
| 4.8 | `runner.rs:483-497` `spawn_adopted_monitor` | 仅按 PID 判断存活，PID 被系统复用后会一直认为接管的 run 还在运行 | 低 |

---

## 优先级建议

1. 修复问题 1（同步命令/事件处理器里创建窗口 → 死锁）。它同时解释「主页面打不开」和「面板关不掉」，并导致问题 3.2 的状态污染。
2. 修复问题 2 的兜底关闭逻辑与看门狗泄漏。
3. 处理问题 3.1 / 3.3 / 3.6（状态扫描覆盖不到、不刷新、接管会被杀）。
4. 其余按表格顺序择机处理。

## 未验证项

- 没有启动应用并实际点击，死锁是依据 Tauri 文档与调用栈推出的；托盘菜单、托盘点击、快捷键、single-instance 四个入口是否也会死锁需要运行后确认。
- 问题 3.6 与「adopted 记录后紧跟强杀记录」的关联、3.7 的触发条件均为推断。

---

## 修复状态

已修复（`cargo check` 与 `cargo test` 通过，未做 GUI 点击验证）：
- 问题 1：`show_main`、`close_panel` 改为 `async` 命令；托盘菜单、single-instance、全局快捷键处理器改为经 `windows::open_main` / `open_panel` 在工作线程里创建窗口；`toggle_panel` 同理。`ensure_main` 用 `close_panel` 代替 `hide_panel`（已删除），不再留下隐藏的旧面板。
- 问题 2：看门狗按面板代次退出，不再泄漏；约 5 秒内始终未获得焦点则关闭面板。
- 问题 3.4：`adopt_external` 的已管理 PID 集合改为 `run.pids`。
- 问题 3.5：Python 白名单加入 `pythonw.exe`。
- 问题 4.2：移除 `apply_hotkey` 上无效的 `#[tauri::command]`。
- 问题 4.3：`update_settings` 先完成唤醒与 `settings-changed` 事件，再返回自启动/快捷键的错误。

未修改（需要产品决策或改动较大）：3.1、3.2、3.3、3.6、3.7、4.1、4.4–4.8。

### 第二轮（排除 3.3、3.6，按要求不修改）

工作区里 3.1、3.2、3.7、4.1、4.4、4.5、4.7、4.8 的改动已经存在（非本轮编辑产生，`git diff` 可见），本轮只核对并验证编译和测试：
- 3.1：编辑对话框已有「进程匹配」的说明文字，提示启动器脚本需要填写常驻进程的命令行特征。
- 3.2：`list_scripts` 用 `state.runs` 的存活句柄校正状态，没有句柄的 running/starting/stopping 记为 stopped。
- 3.7：`start_with` 仅在 `max_instances <= 1` 且无活动实例时才接管已有进程。
- 4.1：`ensure_next` 对已有活动实例的 loop 不再立即补排，避免 skip 策略下的重复启动与刷日志。
- 4.4：debug 构建不再写入开机自启项（已存在的旧注册表项需手动关闭再开启一次设置来清除）。
- 4.5：启动时遗留的 running 记录改记为 `stopped`（`stop_reason=app_exit`），不再标成失败。
- 4.7：移除 `panic = "abort"`，并用 `AdoptingReset` 在 unwind 时复位 `adopting`。
- 4.8：被接管进程的存活判断加入进程创建时间，避免 PID 复用。
- 4.6：`hide_panel` 已删除，面板改为销毁重建，每次都会重新定位。

验证：`cargo check` 无警告，`cargo test` 21 项通过。仍未做 GUI 点击验证。

---

## 5. 确认对话框失效，且样式与应用不一致（已修复）

**位置**：`App.svelte`（移除脚本）、`lib/History.svelte`（清空历史）、`lib/Schedules.svelte`（删除计划）、`lib/Triggers.svelte`（删除触发器）都使用 `window.confirm()`。

**原因**：Tauri 的 WebView 里原生 `confirm()` 不能可靠弹出，调用后直接返回 `false`，所以点击删除/清空没有任何反应；即使弹出，也是系统样式，与应用的弹窗不一致。

**修复**：新增 `lib/confirm.ts`（`confirmDialog()` 返回 `Promise<boolean>`）和 `lib/ConfirmDialog.svelte`（复用 `.modal` / `.btn` / `.btn-danger` 样式，Esc 或点击遮罩取消，打开时聚焦确认按钮），在 `App.svelte` 挂载一次，四处调用改为 `await confirmDialog({...})`，破坏性操作使用危险按钮样式。

**验证**：`vite build` 与 `svelte-check`（0 错误 0 警告）通过。未在运行的应用里点击确认弹窗。

---

## 6. 检测运行时环境时黑色终端窗口闪过（已修复）

**位置**:`runtime.rs` 的 `wsl_distros()`。

**原因**:`wsl.exe -l -q` 是控制台程序,启动时没有设置 `CREATE_NO_WINDOW`,检测运行时会弹出一个黑窗。同文件的 `run_capture` 已设置该标志,只有这一处漏了。其余子进程(`runner.rs` 的脚本启动、`spawn_opener` 打开 GUI 程序)不受影响。

**修复**:给该命令加 `.creation_flags(0x08000000)`。

**验证**:`cargo check` 无警告,`cargo test` 21 项通过。未在运行的应用里观察是否还闪窗。
