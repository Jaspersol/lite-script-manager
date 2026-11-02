use crate::db;
use crate::models::{LogEvent, RunStateEvent, Script};
use crate::runtime;
use crate::state::AppState;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
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
            AssignProcessToJobObject, CreateJobObjectW, SetInformationJobObject,
            JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
            JobObjectExtendedLimitInformation,
        };
        use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_SET_QUOTA, PROCESS_TERMINATE};

        unsafe {
            let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if job.is_null() {
                return Err(format!("创建 Job Object 失败: {}", std::io::Error::last_os_error()));
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
                return Err(format!("配置 Job Object 失败: {}", std::io::Error::last_os_error()));
            }
            let process = OpenProcess(PROCESS_SET_QUOTA | PROCESS_TERMINATE, 0, pid);
            if process.is_null() {
                CloseHandle(job);
                return Err(format!("打开子进程失败: {}", std::io::Error::last_os_error()));
            }
            let assigned = AssignProcessToJobObject(job, process);
            CloseHandle(process);
            if assigned == 0 {
                CloseHandle(job);
                return Err(format!("将子进程加入 Job Object 失败: {}", std::io::Error::last_os_error()));
            }
            Ok(Self { handle: job as isize })
        }
    }

    pub fn terminate(&self) {
        unsafe {
            windows_sys::Win32::System::JobObjects::TerminateJobObject(self.raw(), 1);
        }
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
            libc::kill(-self.pgid, libc::SIGTERM);
        }
    }
}

#[derive(Clone)]
pub struct RunHandle {
    pub run_id: String,
    pub script_id: String,
    pub pid: u32,
    pub guard: Arc<JobHandle>,
    pub stopped: Arc<AtomicBool>,
    pub log_path: String,
    pub started_at: i64,
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
                .args(["-NoLogo", "-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-File"])
                .arg(&script_path);
        }
        "vbscript" => {
            command.args(["//nologo", "//B", "//E:vbscript"]).arg(&script_path);
        }
        "jscript" => {
            command.args(["//nologo", "//B", "//E:jscript"]).arg(&script_path);
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
        command.env("PYTHONUTF8", "1").env("PYTHONIOENCODING", "utf-8");
    }

    command.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP
        command.creation_flags(0x08000000 | 0x00000200);
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
    loop {
        match lines.next_line().await {
            Ok(Some(line)) => {
                let line = tail_string(&line, 16_384);
                let formatted = format!("[{stream}] {line}\n");
                {
                    let mut file = log.lock().await;
                    let _ = file.write_all(formatted.as_bytes()).await;
                }
                let _ = app.emit(
                    "run-log",
                    LogEvent {
                        run_id: run_id.clone(),
                        script_id: script_id.clone(),
                        stream: stream.to_string(),
                        line,
                    },
                );
            }
            Ok(None) | Err(_) => break,
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
    let exe = runtime::resolve_executable(spec, None)?;
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

    let state = app.state::<AppState>();
    let run_id = Uuid::new_v4().to_string();
    let started_at = db::now_ms();
    let log_dir = state.data_dir.join("logs").join(&script.id);
    std::fs::create_dir_all(&log_dir).map_err(|e| format!("创建日志目录失败: {e}"))?;
    let log_path = log_dir.join(format!("{run_id}.log"));
    let std_log_file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .map_err(|e| format!("创建日志文件失败: {e}"))?;
    let log_file = tokio::fs::File::from_std(std_log_file);

    {
        let conn = state.db.lock().map_err(|_| "数据库锁失败".to_string())?;
        db::insert_run(&conn, &run_id, &script.id, Some(pid), started_at, &log_path.display().to_string())?;
    }

    let stopped = Arc::new(AtomicBool::new(false));
    let handle = RunHandle {
        run_id: run_id.clone(),
        script_id: script.id.clone(),
        pid,
        guard: guard.clone(),
        stopped: stopped.clone(),
        log_path: log_path.display().to_string(),
        started_at,
    };
    state.runs.lock().map_err(|_| "运行状态锁失败".to_string())?.insert(run_id.clone(), handle);

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
        tokio::spawn(async move {
            pump_output(stdout, "stdout", log, app, run_id, script_id).await;
        });
    }
    if let Some(stderr) = child.stderr.take() {
        let app = app.clone();
        let log = log.clone();
        let run_id = run_id.clone();
        let script_id = script.id.clone();
        tokio::spawn(async move {
            pump_output(stderr, "stderr", log, app, run_id, script_id).await;
        });
    }

    let app_for_wait = app.clone();
    let run_id_for_wait = run_id.clone();
    let script_id_for_wait = script.id.clone();
    let stopped_for_wait = stopped.clone();
    tokio::spawn(async move {
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
            let _ = db::finish_run(&conn, &run_id_for_wait, final_status, exit_code, error.as_deref());
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
        let runs = state.runs.lock().map_err(|_| "运行状态锁失败".to_string())?;
        runs.values().filter(|run| run.script_id == script_id).cloned().collect::<Vec<_>>()
    };
    if handles.is_empty() {
        return Err("脚本当前没有运行中的实例".to_string());
    }
    for handle in handles {
        handle.stopped.store(true, Ordering::SeqCst);
        handle.guard.terminate();
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
    }
    Ok(())
}

pub fn stop_all(app: &AppHandle) {
    let state = app.state::<AppState>();
    let handles = {
        let runs = state.runs.lock().ok();
        runs.map(|runs| runs.values().cloned().collect::<Vec<_>>()).unwrap_or_default()
    };
    for handle in handles {
        handle.stopped.store(true, Ordering::SeqCst);
        handle.guard.terminate();
    }
}
