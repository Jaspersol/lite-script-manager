use crate::db;
use crate::models::{LogBatchEvent, LogLine, RunStateEvent, Script};
use crate::runtime;
use crate::state::AppState;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::Mutex as AsyncMutex;
use uuid::Uuid;

#[cfg(windows)]
pub struct JobHandle {
    handle: isize,
}

#[cfg(windows)]
unsafe impl Send for JobHandle {}
#[cfg(windows)]
unsafe impl Sync for JobHandle {}

#[cfg(windows)]
impl JobHandle {
    fn raw(&self) -> windows_sys::Win32::Foundation::HANDLE {
        self.handle as windows_sys::Win32::Foundation::HANDLE
    }

    pub fn attach(pid: u32) -> Result<Self, String> {
        use std::mem::zeroed;
        use windows_sys::Win32::Foundation::CloseHandle;
        use windows_sys::Win32::System::JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
            JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
            SetInformationJobObject,
        };
        use windows_sys::Win32::System::Threading::{
            OpenProcess, PROCESS_SET_QUOTA, PROCESS_TERMINATE,
        };

        unsafe {
            let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if job.is_null() {
                return Err(format!(
                    "创建 Job Object 失败: {}",
                    std::io::Error::last_os_error()
                ));
            }
            let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = zeroed();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            let ok = SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                &info as *const _ as *const core::ffi::c_void,
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            );
            if ok == 0 {
                CloseHandle(job);
                return Err(format!(
                    "配置 Job Object 失败: {}",
                    std::io::Error::last_os_error()
                ));
            }
            let process = OpenProcess(PROCESS_SET_QUOTA | PROCESS_TERMINATE, 0, pid);
            if process.is_null() {
                CloseHandle(job);
                return Err(format!(
                    "打开子进程失败: {}",
                    std::io::Error::last_os_error()
                ));
            }
            let assigned = AssignProcessToJobObject(job, process);
            CloseHandle(process);
            if assigned == 0 {
                CloseHandle(job);
                return Err(format!(
                    "将子进程加入 Job Object 失败: {}",
                    std::io::Error::last_os_error()
                ));
            }
            Ok(Self {
                handle: job as isize,
            })
        }
    }

    pub fn terminate(&self) {
        unsafe {
            windows_sys::Win32::System::JobObjects::TerminateJobObject(self.raw(), 1);
        }
    }

    pub fn request_graceful(&self, pid: u32, runtime_id: &str) {
        if matches!(runtime_id, "autohotkey" | "autoit") {
            post_wm_close(pid);
        }
        // Console runtimes receive CTRL_BREAK_EVENT through their own process group.
        // GUI runtimes may ignore it, so WM_CLOSE above is the primary path for them.
        generate_ctrl_break(pid);
    }
}

#[cfg(windows)]
impl Drop for JobHandle {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(self.raw());
        }
    }
}

#[cfg(windows)]
unsafe extern "system" fn close_window_for_pid(
    hwnd: windows_sys::Win32::Foundation::HWND,
    lparam: windows_sys::Win32::Foundation::LPARAM,
) -> windows_sys::Win32::Foundation::BOOL {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetWindowThreadProcessId, IsWindowVisible, PostMessageW, WM_CLOSE,
    };

    let target_pid = lparam as u32;
    let mut window_pid = 0u32;
    unsafe {
        GetWindowThreadProcessId(hwnd, &mut window_pid);
        if window_pid == target_pid && IsWindowVisible(hwnd) != 0 {
            PostMessageW(hwnd, WM_CLOSE, 0, 0);
            return 0;
        }
    }
    1
}

#[cfg(windows)]
fn post_wm_close(pid: u32) {
    use windows_sys::Win32::UI::WindowsAndMessaging::EnumWindows;
    unsafe {
        EnumWindows(Some(close_window_for_pid), pid as isize);
    }
}

#[cfg(windows)]
fn generate_ctrl_break(pid: u32) {
    use windows_sys::Win32::System::Console::{
        AttachConsole, CTRL_BREAK_EVENT, FreeConsole, GenerateConsoleCtrlEvent,
    };
    unsafe {
        FreeConsole();
        if AttachConsole(pid) != 0 {
            GenerateConsoleCtrlEvent(CTRL_BREAK_EVENT, pid);
            FreeConsole();
        }
    }
}

#[cfg(windows)]
fn resume_process(pid: u32) -> Result<(), String> {
    use std::mem::{size_of, zeroed};
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, TH32CS_SNAPTHREAD, THREADENTRY32, Thread32First, Thread32Next,
    };
    use windows_sys::Win32::System::Threading::{OpenThread, ResumeThread, THREAD_SUSPEND_RESUME};

    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0);
        if snapshot == INVALID_HANDLE_VALUE {
            return Err(format!(
                "创建线程快照失败: {}",
                std::io::Error::last_os_error()
            ));
        }
        let mut entry: THREADENTRY32 = zeroed();
        entry.dwSize = size_of::<THREADENTRY32>() as u32;
        let mut resumed = false;
        if Thread32First(snapshot, &mut entry) != 0 {
            loop {
                if entry.th32OwnerProcessID == pid {
                    let thread = OpenThread(THREAD_SUSPEND_RESUME, 0, entry.th32ThreadID);
                    if !thread.is_null() {
                        if ResumeThread(thread) != u32::MAX {
                            resumed = true;
                        }
                        CloseHandle(thread);
                        if resumed {
                            break;
                        }
                    }
                }
                if Thread32Next(snapshot, &mut entry) == 0 {
                    break;
                }
            }
        }
        CloseHandle(snapshot);
        if resumed {
            Ok(())
        } else {
            Err("无法恢复挂起的子进程线程".to_string())
        }
    }
}

#[cfg(not(windows))]
fn resume_process(_pid: u32) -> Result<(), String> {
    Ok(())
}

#[cfg(unix)]
pub struct JobHandle {
    pgid: i32,
}

#[cfg(unix)]
impl JobHandle {
    pub fn attach(pid: u32) -> Result<Self, String> {
        Ok(Self { pgid: pid as i32 })
    }

    pub fn terminate(&self) {
        unsafe {
            libc::kill(-self.pgid, libc::SIGKILL);
        }
    }

    pub fn request_graceful(&self, _pid: u32, _runtime_id: &str) {
        unsafe {
            libc::kill(-self.pgid, libc::SIGTERM);
        }
    }
}

#[derive(Clone)]
pub struct RunHandle {
    pub run_id: String,
    pub script_id: String,
    pub runtime_id: String,
    pub pid: u32,
    pub guard: Arc<JobHandle>,
    pub stopped: Arc<AtomicBool>,
    pub started_at: i64,
    pub stop_timeout_sec: i64,
}

fn tail_string(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        value.to_string()
    } else {
        value.chars().take(max_chars).collect()
    }
}

fn build_command(script: &Script, exe: &PathBuf) -> Result<tokio::process::Command, String> {
    let spec = runtime::find_spec(&script.runtime_id)
        .ok_or_else(|| format!("未知运行时: {}", script.runtime_id))?;
    let script_path = if spec.id == "shell" {
        script.path.replace('\\', "/")
    } else {
        script.path.clone()
    };

    let mut command = tokio::process::Command::new(exe);
    match spec.id {
        "python" => {
            command.args(["-u", "-X", "utf8"]).arg(&script_path);
        }
        "javascript" => {
            command.arg(&script_path);
        }
        "shell" => {
            command.arg(&script_path);
        }
        "perl" | "php" | "r" | "lua" => {
            command.arg(&script_path);
        }
        "batch" => {
            command.args(["/d", "/s", "/c", "call"]).arg(&script_path);
        }
        "powershell" => {
            command
                .args([
                    "-NoLogo",
                    "-NoProfile",
                    "-NonInteractive",
                    "-ExecutionPolicy",
                    "Bypass",
                    "-File",
                ])
                .arg(&script_path);
        }
        "vbscript" => {
            command
                .args(["//nologo", "//B", "//E:vbscript"])
                .arg(&script_path);
        }
        "jscript" => {
            command
                .args(["//nologo", "//B", "//E:jscript"])
                .arg(&script_path);
        }
        "wsf" => {
            command.args(["//nologo", "//B"]).arg(&script_path);
        }
        "autohotkey" => {
            command.arg(&script_path);
        }
        "autoit" => {
            command.arg("/ErrorStdOut").arg(&script_path);
        }
        _ => return Err(format!("暂不支持运行 {}", spec.name)),
    }
    command.args(&script.args);

    if let Some(cwd) = &script.cwd {
        let cwd = PathBuf::from(cwd);
        if cwd.exists() {
            command.current_dir(cwd);
        }
    } else if let Some(parent) = PathBuf::from(&script.path).parent() {
        command.current_dir(parent);
    }
    for (key, value) in &script.env {
        command.env(key, value);
    }
    if spec.id == "python" {
        command
            .env("PYTHONUTF8", "1")
            .env("PYTHONIOENCODING", "utf-8");
    }

    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        // CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP | CREATE_SUSPENDED
        command.creation_flags(0x08000000 | 0x00000200 | 0x00000004);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        unsafe {
            command.pre_exec(|| {
                libc::setsid();
                Ok(())
            });
        }
    }
    Ok(command)
}

fn flush_log_batch(app: &AppHandle, run_id: &str, script_id: &str, batch: &mut Vec<LogLine>) {
    if batch.is_empty() {
        return;
    }
    let _ = app.emit(
        "run-log-batch",
        LogBatchEvent {
            run_id: run_id.to_string(),
            script_id: script_id.to_string(),
            lines: std::mem::take(batch),
        },
    );
}

async fn pump_output<R>(
    reader: R,
    stream: &'static str,
    log: Arc<AsyncMutex<tokio::fs::File>>,
    app: AppHandle,
    run_id: String,
    script_id: String,
) where
    R: tokio::io::AsyncRead + Unpin,
{
    let mut lines = BufReader::new(reader).lines();
    let mut ticker = tokio::time::interval(Duration::from_millis(100));
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut batch: Vec<LogLine> = Vec::new();

    loop {
        tokio::select! {
            _ = ticker.tick() => {
                flush_log_batch(&app, &run_id, &script_id, &mut batch);
            }
            result = lines.next_line() => {
                match result {
                    Ok(Some(line)) => {
                        let line = tail_string(&line, 16_384);
                        let formatted = format!("[{stream}] {line}\n");
                        {
                            let mut file = log.lock().await;
                            let _ = file.write_all(formatted.as_bytes()).await;
                        }
                        batch.push(LogLine { stream: stream.to_string(), line });
                        if batch.len() >= 64 {
                            flush_log_batch(&app, &run_id, &script_id, &mut batch);
                        }
                    }
                    Ok(None) | Err(_) => {
                        flush_log_batch(&app, &run_id, &script_id, &mut batch);
                        break;
                    }
                }
            }
        }
    }
}

pub fn start(app: &AppHandle, script: Script) -> Result<RunStateEvent, String> {
    if !script.enabled {
        return Err("脚本已禁用".to_string());
    }
    if !PathBuf::from(&script.path).exists() {
        return Err(format!("脚本文件不存在: {}", script.path));
    }
    let spec = runtime::find_spec(&script.runtime_id)
        .ok_or_else(|| format!("未知运行时: {}", script.runtime_id))?;
    let exe =
        runtime::resolve_executable(spec, script.interpreter_path.as_deref(), Some(&script.path))?;
    let mut command = build_command(&script, &exe)?;
    let mut child = command.spawn().map_err(|e| format!("启动进程失败: {e}"))?;
    let pid = child.id().ok_or_else(|| "无法获取子进程 PID".to_string())?;

    let guard = match JobHandle::attach(pid) {
        Ok(guard) => Arc::new(guard),
        Err(error) => {
            let _ = child.start_kill();
            return Err(error);
        }
    };
    if let Err(error) = resume_process(pid) {
        guard.terminate();
        let _ = child.start_kill();
        return Err(error);
    }

    let state = app.state::<AppState>();
    let run_id = Uuid::new_v4().to_string();
    let started_at = db::now_ms();
    let log_dir = state.data_dir.join("logs").join(&script.id);
    if let Err(error) = std::fs::create_dir_all(&log_dir) {
        guard.terminate();
        return Err(format!("创建日志目录失败: {error}"));
    }
    let log_path = log_dir.join(format!("{run_id}.log"));
    let std_log_file = match std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
    {
        Ok(file) => file,
        Err(error) => {
            guard.terminate();
            return Err(format!("创建日志文件失败: {error}"));
        }
    };
    let log_file = tokio::fs::File::from_std(std_log_file);

    {
        let conn = state.db.lock().map_err(|_| "数据库锁失败".to_string())?;
        if let Err(error) = db::insert_run(
            &conn,
            &run_id,
            &script.id,
            Some(pid),
            started_at,
            &log_path.display().to_string(),
        ) {
            drop(conn);
            guard.terminate();
            return Err(error);
        }
    }

    let stopped = Arc::new(AtomicBool::new(false));
    let handle = RunHandle {
        run_id: run_id.clone(),
        script_id: script.id.clone(),
        runtime_id: script.runtime_id.clone(),
        pid,
        guard: guard.clone(),
        stopped: stopped.clone(),
        started_at,
        stop_timeout_sec: script.stop_timeout_sec.max(1),
    };
    if let Err(error) = state
        .runs
        .lock()
        .map_err(|_| "运行状态锁失败".to_string())
        .map(|mut runs| {
            runs.insert(run_id.clone(), handle);
        })
    {
        guard.terminate();
        return Err(error);
    }

    let event = RunStateEvent {
        run_id: run_id.clone(),
        script_id: script.id.clone(),
        status: "running".into(),
        exit_code: None,
        error: None,
        started_at,
        ended_at: None,
    };
    let _ = app.emit("run-state", event.clone());

    let log = Arc::new(AsyncMutex::new(log_file));
    if let Some(stdout) = child.stdout.take() {
        let app = app.clone();
        let log = log.clone();
        let run_id = run_id.clone();
        let script_id = script.id.clone();
        tauri::async_runtime::spawn(async move {
            pump_output(stdout, "stdout", log, app, run_id, script_id).await;
        });
    }
    if let Some(stderr) = child.stderr.take() {
        let app = app.clone();
        let log = log.clone();
        let run_id = run_id.clone();
        let script_id = script.id.clone();
        tauri::async_runtime::spawn(async move {
            pump_output(stderr, "stderr", log, app, run_id, script_id).await;
        });
    }

    let app_for_wait = app.clone();
    let run_id_for_wait = run_id.clone();
    let script_id_for_wait = script.id.clone();
    let stopped_for_wait = stopped.clone();
    tauri::async_runtime::spawn(async move {
        let wait_result = child.wait().await;
        let was_stopped = stopped_for_wait.load(Ordering::SeqCst);
        let (exit_code, wait_error) = match wait_result {
            Ok(status) => (status.code(), None),
            Err(error) => (None, Some(format!("等待进程结束失败: {error}"))),
        };
        let final_status = if was_stopped {
            "stopped"
        } else if exit_code == Some(0) {
            "exited"
        } else {
            "failed"
        };
        let error = wait_error.or_else(|| {
            if final_status == "failed" {
                Some(format!("进程退出码 {}", exit_code.unwrap_or(-1)))
            } else {
                None
            }
        });
        let state = app_for_wait.state::<AppState>();
        if let Ok(conn) = state.db.lock() {
            let _ = db::finish_run(
                &conn,
                &run_id_for_wait,
                final_status,
                exit_code,
                error.as_deref(),
            );
        }
        if let Ok(mut runs) = state.runs.lock() {
            runs.remove(&run_id_for_wait);
        }
        let _ = app_for_wait.emit(
            "run-state",
            RunStateEvent {
                run_id: run_id_for_wait,
                script_id: script_id_for_wait,
                status: final_status.to_string(),
                exit_code,
                error,
                started_at,
                ended_at: Some(db::now_ms()),
            },
        );
    });

    Ok(event)
}

pub fn stop(app: &AppHandle, script_id: &str) -> Result<(), String> {
    let state = app.state::<AppState>();
    let handles = {
        let runs = state
            .runs
            .lock()
            .map_err(|_| "运行状态锁失败".to_string())?;
        runs.values()
            .filter(|run| run.script_id == script_id)
            .cloned()
            .collect::<Vec<_>>()
    };
    if handles.is_empty() {
        return Err("脚本当前没有运行中的实例".to_string());
    }
    for handle in handles {
        handle.stopped.store(true, Ordering::SeqCst);
        handle
            .guard
            .request_graceful(handle.pid, &handle.runtime_id);
        let _ = app.emit(
            "run-state",
            RunStateEvent {
                run_id: handle.run_id.clone(),
                script_id: handle.script_id.clone(),
                status: "stopping".into(),
                exit_code: None,
                error: None,
                started_at: handle.started_at,
                ended_at: None,
            },
        );

        // Give the process a chance to exit cleanly. The wait task removes the
        // run from state as soon as it finishes; only surviving runs are forced.
        let app_for_timer = app.clone();
        let guard = handle.guard.clone();
        let run_id = handle.run_id.clone();
        let timeout = handle.stop_timeout_sec.max(1) as u64;
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(Duration::from_secs(timeout)).await;
            let still_running = app_for_timer
                .state::<AppState>()
                .runs
                .lock()
                .map(|runs| runs.contains_key(&run_id))
                .unwrap_or(false);
            if still_running {
                guard.terminate();
            }
        });
    }
    Ok(())
}

pub fn stop_all(app: &AppHandle) {
    let state = app.state::<AppState>();
    let handles = {
        let runs = state.runs.lock().ok();
        runs.map(|runs| runs.values().cloned().collect::<Vec<_>>())
            .unwrap_or_default()
    };
    for handle in handles {
        handle.stopped.store(true, Ordering::SeqCst);
        handle.guard.terminate();
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::resume_process;
    use std::os::windows::process::CommandExt;

    #[test]
    fn resumes_a_suspended_process() {
        let mut command = std::process::Command::new("cmd.exe");
        command
            .args(["/d", "/c", "exit", "0"])
            .creation_flags(0x08000000 | 0x00000004);
        let mut child = command.spawn().expect("spawn suspended child");
        let pid = child.id();
        if let Err(error) = resume_process(pid) {
            let _ = child.kill();
            panic!("{error}");
        }
        let status = child.wait().expect("wait for child");
        assert!(status.success());
    }
}
