use crate::db;
use crate::logging::{self, LogRing};
use crate::metrics;
use crate::models::{LifecycleEvent, LogBatchEvent, LogLine, RunStateEvent, Script, StartOptions};
use crate::runtime;
use crate::settings;
use crate::state::AppState;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::Mutex as StdMutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, BufWriter};
use tokio::sync::Mutex as AsyncMutex;
use uuid::Uuid;

const LOG_RING_BYTES: usize = 256 * 1024;
const MAX_LINE_CHARS: usize = 16_384;

#[cfg(windows)]
pub struct JobHandle {
    handle: isize,
}

#[cfg(windows)]
unsafe impl Send for JobHandle {}
#[cfg(windows)]
unsafe impl Sync for JobHandle {}

#[cfg(windows)]
fn create_job_object() -> Result<isize, String> {
    use std::mem::zeroed;
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::JobObjects::{
        CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JobObjectExtendedLimitInformation, SetInformationJobObject,
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
        Ok(job as isize)
    }
}

#[cfg(windows)]
fn assign_process_to_job(job: isize, pid: u32) -> Result<(), String> {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::JobObjects::AssignProcessToJobObject;
    use windows_sys::Win32::System::Threading::{
        OpenProcess, PROCESS_SET_QUOTA, PROCESS_TERMINATE,
    };

    unsafe {
        let process = OpenProcess(PROCESS_SET_QUOTA | PROCESS_TERMINATE, 0, pid);
        if process.is_null() {
            return Err(format!(
                "打开子进程失败: {}",
                std::io::Error::last_os_error()
            ));
        }
        let assigned = AssignProcessToJobObject(job as _, process);
        CloseHandle(process);
        if assigned == 0 {
            Err(format!(
                "将子进程加入 Job Object 失败: {}",
                std::io::Error::last_os_error()
            ))
        } else {
            Ok(())
        }
    }
}

#[cfg(windows)]
impl JobHandle {
    fn raw(&self) -> windows_sys::Win32::Foundation::HANDLE {
        self.handle as windows_sys::Win32::Foundation::HANDLE
    }

    pub fn attach(pid: u32) -> Result<Self, String> {
        let handle = create_job_object()?;
        if let Err(error) = assign_process_to_job(handle, pid) {
            unsafe {
                windows_sys::Win32::Foundation::CloseHandle(
                    handle as windows_sys::Win32::Foundation::HANDLE,
                );
            }
            return Err(error);
        }
        Ok(Self { handle })
    }

    pub fn attach_many(pids: &[u32]) -> Result<Self, String> {
        let handle = create_job_object()?;
        let mut assigned = 0usize;
        for pid in pids {
            if assign_process_to_job(handle, *pid).is_ok() {
                assigned += 1;
            }
        }
        if assigned == 0 {
            unsafe {
                windows_sys::Win32::Foundation::CloseHandle(
                    handle as windows_sys::Win32::Foundation::HANDLE,
                );
            }
            return Err("没有进程可以加入 Job Object".to_string());
        }
        Ok(Self { handle })
    }

    pub fn terminate(&self) {
        unsafe {
            windows_sys::Win32::System::JobObjects::TerminateJobObject(self.raw(), 1);
        }
    }

    pub fn active_processes(&self) -> u32 {
        use std::mem::{size_of, zeroed};
        use windows_sys::Win32::System::JobObjects::{
            JOBOBJECT_BASIC_ACCOUNTING_INFORMATION, JobObjectBasicAccountingInformation,
            QueryInformationJobObject,
        };

        unsafe {
            let mut info: JOBOBJECT_BASIC_ACCOUNTING_INFORMATION = zeroed();
            let ok = QueryInformationJobObject(
                self.raw(),
                JobObjectBasicAccountingInformation,
                &mut info as *mut _ as *mut core::ffi::c_void,
                size_of::<JOBOBJECT_BASIC_ACCOUNTING_INFORMATION>() as u32,
                std::ptr::null_mut(),
            );
            if ok == 0 { 0 } else { info.ActiveProcesses }
        }
    }

    pub fn request_graceful(&self, pid: u32, runtime_id: &str) {
        if matches!(runtime_id, "autohotkey" | "autoit") {
            post_wm_close(pid);
        }
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

#[cfg(windows)]
pub(crate) fn process_command_line(pid: u32) -> Option<String> {
    use std::mem::size_of;
    use windows_sys::Wdk::System::Threading::{
        NtQueryInformationProcess, ProcessCommandLineInformation,
    };
    use windows_sys::Win32::Foundation::{CloseHandle, UNICODE_STRING};
    use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};

    unsafe {
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if process.is_null() {
            return None;
        }
        let mut buffer = vec![0u64; 8192];
        let mut returned = 0u32;
        let status = NtQueryInformationProcess(
            process,
            ProcessCommandLineInformation,
            buffer.as_mut_ptr() as *mut core::ffi::c_void,
            (buffer.len() * size_of::<u64>()) as u32,
            &mut returned,
        );
        if status < 0 {
            CloseHandle(process);
            return None;
        }
        let unicode = &*(buffer.as_ptr() as *const UNICODE_STRING);
        let chars = unicode.Length as usize / size_of::<u16>();
        let limit = (buffer.len() * size_of::<u64>()) / size_of::<u16>();
        let result = if unicode.Buffer.is_null() || chars == 0 || chars > limit {
            None
        } else {
            Some(String::from_utf16_lossy(std::slice::from_raw_parts(
                unicode.Buffer,
                chars,
            )))
        };
        CloseHandle(process);
        result
    }
}

#[cfg(windows)]
pub(crate) fn snapshot_processes() -> Vec<(u32, u32, String)> {
    use std::mem::{size_of, zeroed};
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
        TH32CS_SNAPPROCESS,
    };

    let mut processes = Vec::new();
    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snapshot == INVALID_HANDLE_VALUE {
            return processes;
        }
        let mut entry: PROCESSENTRY32W = zeroed();
        entry.dwSize = size_of::<PROCESSENTRY32W>() as u32;
        if Process32FirstW(snapshot, &mut entry) != 0 {
            loop {
                let name_len = entry
                    .szExeFile
                    .iter()
                    .position(|ch| *ch == 0)
                    .unwrap_or(entry.szExeFile.len());
                let exe = String::from_utf16_lossy(&entry.szExeFile[..name_len]);
                processes.push((entry.th32ProcessID, entry.th32ParentProcessID, exe));
                if Process32NextW(snapshot, &mut entry) == 0 {
                    break;
                }
            }
        }
        CloseHandle(snapshot);
    }
    processes
}

#[cfg(not(windows))]
pub(crate) fn snapshot_processes() -> Vec<(u32, u32, String)> {
    Vec::new()
}

#[cfg(windows)]
pub(crate) fn process_create_time_ms(pid: u32) -> Option<i64> {
    use std::mem::zeroed;
    use windows_sys::Win32::Foundation::{CloseHandle, FILETIME};
    use windows_sys::Win32::System::Threading::{
        GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    unsafe {
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if process.is_null() {
            return None;
        }
        let (mut creation, mut exit, mut kernel, mut user): (
            FILETIME,
            FILETIME,
            FILETIME,
            FILETIME,
        ) = (zeroed(), zeroed(), zeroed(), zeroed());
        let ok = GetProcessTimes(process, &mut creation, &mut exit, &mut kernel, &mut user);
        CloseHandle(process);
        if ok == 0 {
            return None;
        }
        let value = ((creation.dwHighDateTime as u64) << 32) | creation.dwLowDateTime as u64;
        Some((value / 10_000) as i64 - 11_644_473_600_000)
    }
}

#[cfg(not(windows))]
pub(crate) fn process_create_time_ms(_pid: u32) -> Option<i64> {
    None
}

#[cfg(windows)]
pub fn process_alive(pid: u32) -> bool {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};

    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle.is_null() {
            false
        } else {
            CloseHandle(handle);
            true
        }
    }
}

#[cfg(not(windows))]
pub fn process_alive(_pid: u32) -> bool {
    false
}

#[cfg(windows)]
fn terminate_process_trees(roots: &[u32]) {
    use std::collections::{HashMap, HashSet};
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_TERMINATE, TerminateProcess};

    if roots.is_empty() {
        return;
    }
    let mut children: HashMap<u32, Vec<u32>> = HashMap::new();
    for (pid, parent, _exe) in snapshot_processes() {
        children.entry(parent).or_default().push(pid);
    }
    let mut targets = HashSet::new();
    let mut stack = roots.to_vec();
    while let Some(pid) = stack.pop() {
        if !targets.insert(pid) {
            continue;
        }
        if let Some(kids) = children.get(&pid) {
            stack.extend(kids);
        }
    }
    let current = std::process::id();
    for pid in targets {
        if pid == current {
            continue;
        }
        unsafe {
            let handle = OpenProcess(PROCESS_TERMINATE, 0, pid);
            if !handle.is_null() {
                TerminateProcess(handle, 1);
                CloseHandle(handle);
            }
        }
    }
}

#[cfg(not(windows))]
fn terminate_process_trees(_roots: &[u32]) {}

fn normalized_match(value: &str) -> String {
    let value = value.trim();
    let value = value.strip_prefix(r"\\?\").unwrap_or(value);
    value.replace('/', "\\").to_lowercase()
}

fn executable_allowed(script: &Script, exe: &str) -> bool {
    let exe = exe.to_ascii_lowercase();
    if let Some(spec) = runtime::find_spec(&script.runtime_id) {
        if spec
            .executables
            .iter()
            .any(|candidate| candidate.eq_ignore_ascii_case(&exe))
        {
            return true;
        }
    } else {
        return true;
    }
    script
        .interpreter_path
        .as_deref()
        .and_then(|path| std::path::Path::new(path).file_name())
        .and_then(|name| name.to_str())
        .map(|name| name.eq_ignore_ascii_case(&exe))
        .unwrap_or(false)
}

#[cfg(windows)]
fn spawn_adopted_monitor(app: AppHandle, handle: RunHandle) {
    tauri::async_runtime::spawn(async move {
        while handle.guard.active_processes() > 0
            || handle.pids.iter().any(|pid| process_alive(*pid))
        {
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        let status = if handle.stopped.load(Ordering::SeqCst) {
            "stopped"
        } else {
            "exited"
        };
        finish_run(&app, &handle, status, None, None);
    });
}

#[cfg(windows)]
pub fn adopt_external(app: &AppHandle) -> Result<usize, String> {
    let state = app.state::<AppState>();
    if state.adopting.swap(true, Ordering::SeqCst) {
        return Ok(0);
    }

    let result = (|| -> Result<usize, String> {
        let scripts = {
            let conn = state.db.lock().map_err(|_| "数据库锁失败".to_string())?;
            db::list_scripts(&conn)?
        };
        let processes = snapshot_processes()
            .into_iter()
            .map(|(pid, parent, exe)| {
                let normalized = normalized_match(&exe);
                (pid, parent, exe, normalized)
            })
            .collect::<Vec<_>>();
        if processes.is_empty() {
            return Ok(0);
        }

        let (mut managed_pids, managed_scripts) = {
            let runs = state
                .runs
                .lock()
                .map_err(|_| "运行状态锁失败".to_string())?;
            let pids = runs.values().map(|run| run.pid).collect::<HashSet<_>>();
            let scripts = runs
                .values()
                .map(|run| run.script_id.clone())
                .collect::<HashSet<_>>();
            (pids, scripts)
        };

        let current_pid = std::process::id();
        let mut command_lines: HashMap<u32, String> = HashMap::new();
        let mut adopted = 0usize;
        for script in scripts {
            if managed_scripts.contains(&script.id) {
                continue;
            }

            let pattern = script
                .process_match
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty());
            let needle = match pattern {
                Some(value) => normalized_match(value),
                None => normalized_match(&script.path),
            };
            if needle.is_empty() {
                continue;
            }

            let mut pids = Vec::new();
            for (pid, _parent, exe, normalized_exe) in &processes {
                if *pid == current_pid || managed_pids.contains(pid) {
                    continue;
                }
                if pattern.is_none() && !executable_allowed(&script, exe) {
                    continue;
                }
                if normalized_exe.contains(&needle) {
                    pids.push(*pid);
                    continue;
                }
                let command_line = command_lines.entry(*pid).or_insert_with(|| {
                    normalized_match(&process_command_line(*pid).unwrap_or_default())
                });
                if command_line.contains(&needle) {
                    pids.push(*pid);
                }
            }
            if pids.is_empty() {
                continue;
            }

            let guard = match JobHandle::attach_many(&pids) {
                Ok(guard) => Arc::new(guard),
                Err(error) => {
                    logging::app_log("WARN", &format!("接管 {} 失败: {error}", script.name));
                    continue;
                }
            };
            let run_id = Uuid::new_v4().to_string();
            let started_at = db::now_ms();
            {
                let conn = state.db.lock().map_err(|_| "数据库锁失败".to_string())?;
                if let Err(error) = db::insert_run(
                    &conn,
                    &run_id,
                    &script.id,
                    pids.first().copied(),
                    started_at,
                    "",
                    "adopted",
                    None,
                    None,
                ) {
                    guard.terminate();
                    logging::app_log("ERROR", &format!("接管写入失败: {error}"));
                    continue;
                }
            }

            let stopped = Arc::new(AtomicBool::new(false));
            let handle = RunHandle {
                run_id: run_id.clone(),
                script_id: script.id.clone(),
                runtime_id: script.runtime_id.clone(),
                pid: pids[0],
                pids: pids.clone(),
                guard: guard.clone(),
                stopped: stopped.clone(),
                stop_reason: Arc::new(StdMutex::new(None)),
                started_at,
                stop_timeout_sec: script.stop_timeout_sec.max(1),
                stop_mode: script.stop_mode.clone(),
                timeout_sec: None,
                success_codes: script.success_codes.clone(),
                source: "adopted".into(),
                source_id: None,
                ring: Arc::new(LogRing::new(LOG_RING_BYTES)),
                peak_memory: Arc::new(AtomicU64::new(0)),
                cpu_time: Arc::new(AtomicU64::new(0)),
                log_bytes: Arc::new(AtomicU64::new(0)),
            };
            {
                let mut runs = state
                    .runs
                    .lock()
                    .map_err(|_| "运行状态锁失败".to_string())?;
                runs.insert(run_id.clone(), handle.clone());
            }
            for pid in &pids {
                managed_pids.insert(*pid);
            }

            let _ = app.emit(
                "run-state",
                RunStateEvent {
                    run_id: run_id.clone(),
                    script_id: script.id.clone(),
                    status: "running".into(),
                    exit_code: None,
                    error: None,
                    started_at,
                    ended_at: None,
                    source: Some("adopted".into()),
                    source_id: None,
                },
            );
            spawn_adopted_monitor(app.clone(), handle);
            adopted += 1;
        }
        Ok(adopted)
    })();

    state.adopting.store(false, Ordering::SeqCst);
    result
}

#[cfg(not(windows))]
pub fn adopt_external(_app: &AppHandle) -> Result<usize, String> {
    Ok(0)
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

    pub fn active_processes(&self) -> u32 {
        unsafe { if libc::kill(-self.pgid, 0) == 0 { 1 } else { 0 } }
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
    pub pids: Vec<u32>,
    pub guard: Arc<JobHandle>,
    pub stopped: Arc<AtomicBool>,
    pub stop_reason: Arc<StdMutex<Option<String>>>,
    pub started_at: i64,
    pub stop_timeout_sec: i64,
    pub stop_mode: String,
    pub timeout_sec: Option<i64>,
    pub success_codes: String,
    pub source: String,
    pub source_id: Option<String>,
    pub ring: Arc<LogRing>,
    pub peak_memory: Arc<AtomicU64>,
    pub cpu_time: Arc<AtomicU64>,
    pub log_bytes: Arc<AtomicU64>,
}

fn tail_string(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        value.to_string()
    } else {
        value
            .chars()
            .skip(value.chars().count().saturating_sub(max_chars))
            .collect()
    }
}

fn success_code_allowed(expression: &str, code: i32) -> bool {
    let expression = expression.trim();
    if expression.is_empty() || expression == "*" {
        return true;
    }
    for part in expression.split(',') {
        let part = part.trim();
        if let Some((start, end)) = part.split_once('-') {
            let start = start.trim().parse::<i32>().ok();
            let end = end.trim().parse::<i32>().ok();
            if let (Some(start), Some(end)) = (start, end)
                && code >= start
                && code <= end
            {
                return true;
            }
        } else if part.parse::<i32>().ok() == Some(code) {
            return true;
        }
    }
    false
}

fn preflight(app: &AppHandle, script: &Script) -> Result<(), String> {
    if !script.enabled {
        return Err("脚本已禁用".to_string());
    }
    let path = PathBuf::from(&script.path);
    if !path.is_file() {
        return Err(format!("脚本文件不存在: {}", script.path));
    }
    let state = app.state::<AppState>();
    let (allowed_roots, verify_hash) = {
        let conn = state.db.lock().map_err(|_| "数据库锁失败".to_string())?;
        (
            settings::get(&conn, "allowed_roots").unwrap_or_default(),
            settings::get_bool(&conn, "verify_script_hash", false),
        )
    };
    if !logging::path_allowed(&path, &allowed_roots) {
        return Err("脚本路径不在允许的根目录中".to_string());
    }
    if let Some(cwd) = &script.cwd {
        let cwd = PathBuf::from(cwd);
        if !cwd.is_dir() {
            return Err(format!("工作目录不存在: {}", cwd.display()));
        }
    }
    if script.args.iter().any(|arg| arg.contains('\0')) {
        return Err("参数中包含非法空字符".to_string());
    }
    if script
        .env
        .keys()
        .any(|key| key.is_empty() || key.contains('\0') || key.contains('='))
    {
        return Err("环境变量名称无效".to_string());
    }
    if verify_hash && let Some(expected) = &script.content_hash {
        let actual = logging::sha256_file(&path)?;
        if !actual.eq_ignore_ascii_case(expected) {
            return Err("脚本内容哈希不匹配，已拒绝运行".to_string());
        }
    }
    Ok(())
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
        "javascript" | "shell" | "perl" | "php" | "r" | "lua" | "autohotkey" => {
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
        "autoit" => {
            command.arg("/ErrorStdOut").arg(&script_path);
        }
        _ => return Err(format!("暂不支持运行 {}", spec.name)),
    }
    command.args(&script.args);

    if let Some(cwd) = &script.cwd {
        command.current_dir(PathBuf::from(cwd));
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

struct OutputSink {
    log: Arc<AsyncMutex<BufWriter<tokio::fs::File>>>,
    app: AppHandle,
    run_id: String,
    script_id: String,
    ring: Arc<LogRing>,
    log_bytes: Arc<AtomicU64>,
}

async fn pump_output<R>(reader: R, stream: &'static str, sink: OutputSink)
where
    R: tokio::io::AsyncRead + Unpin,
{
    let OutputSink {
        log,
        app,
        run_id,
        script_id,
        ring,
        log_bytes,
    } = sink;
    let mut reader = BufReader::new(reader);
    let mut ticker = tokio::time::interval(Duration::from_millis(100));
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut batch: Vec<LogLine> = Vec::new();

    loop {
        let mut bytes = Vec::new();
        tokio::select! {
            _ = ticker.tick() => {
                let _ = log.lock().await.flush().await;
                if app.state::<AppState>().log_streaming.load(Ordering::Relaxed) {
                    flush_log_batch(&app, &run_id, &script_id, &mut batch);
                } else {
                    batch.clear();
                }
            }
            result = reader.read_until(b'\n', &mut bytes) => {
                match result {
                    Ok(0) => {
                        if app.state::<AppState>().log_streaming.load(Ordering::Relaxed) {
                            flush_log_batch(&app, &run_id, &script_id, &mut batch);
                        }
                        break;
                    }
                    Ok(_) => {
                        while bytes.last().is_some_and(|value| *value == b'\n' || *value == b'\r') {
                            bytes.pop();
                        }
                        let line = tail_string(&logging::decode_output(&bytes), MAX_LINE_CHARS);
                        let formatted = format!("[{stream}] {line}\n");
                        {
                            let mut file = log.lock().await;
                            if file.write_all(formatted.as_bytes()).await.is_ok() {
                                log_bytes.fetch_add(formatted.len() as u64, Ordering::Relaxed);
                            }
                        }
                        if app.state::<AppState>().log_streaming.load(Ordering::Relaxed) {
                            ring.push(LogLine { stream: stream.to_string(), line: line.clone() });
                            batch.push(LogLine { stream: stream.to_string(), line });
                            if batch.len() >= 64 {
                                flush_log_batch(&app, &run_id, &script_id, &mut batch);
                            }
                        } else {
                            ring.push(LogLine { stream: stream.to_string(), line });
                        }
                    }
                    Err(_) => {
                        if app.state::<AppState>().log_streaming.load(Ordering::Relaxed) {
                            flush_log_batch(&app, &run_id, &script_id, &mut batch);
                        }
                        break;
                    }
                }
            }
        }
    }
    let _ = log.lock().await.flush().await;
}

fn finish_run(
    app: &AppHandle,
    run: &RunHandle,
    status: &str,
    exit_code: Option<i32>,
    error: Option<String>,
) {
    let final_metrics = metrics::process_metrics(&run.pids);
    let peak = run
        .peak_memory
        .fetch_max(final_metrics.memory_bytes, Ordering::Relaxed)
        .max(final_metrics.memory_bytes);
    let cpu = run
        .cpu_time
        .fetch_max(final_metrics.cpu_time_ms, Ordering::Relaxed)
        .max(final_metrics.cpu_time_ms);
    let reason = run.stop_reason.lock().ok().and_then(|value| value.clone());
    let state = app.state::<AppState>();
    {
        if let Ok(conn) = state.db.lock() {
            let _ = db::finish_run(
                &conn,
                &run.run_id,
                status,
                exit_code,
                error.as_deref(),
                reason.as_deref(),
                Some(peak as i64),
                Some(cpu as i64),
                Some(run.log_bytes.load(Ordering::Relaxed) as i64),
            );
        }
    }
    logging::auto_prune_logs(&state.db, &state.data_dir);
    if let Ok(mut runs) = state.runs.lock() {
        runs.remove(&run.run_id);
    }
    let _ = app.emit(
        "run-state",
        RunStateEvent {
            run_id: run.run_id.clone(),
            script_id: run.script_id.clone(),
            status: status.to_string(),
            exit_code,
            error,
            started_at: run.started_at,
            ended_at: Some(db::now_ms()),
            source: Some(run.source.clone()),
            source_id: run.source_id.clone(),
        },
    );
    let _ = state.lifecycle_tx.send(LifecycleEvent {
        run_id: run.run_id.clone(),
        script_id: run.script_id.clone(),
        status: status.to_string(),
        exit_code,
        source: run.source.clone(),
        source_id: run.source_id.clone(),
    });
}

fn spawn_metrics_sampler(app: AppHandle, handle: RunHandle) {
    tauri::async_runtime::spawn(async move {
        loop {
            {
                let state = app.state::<AppState>();
                let active = state
                    .runs
                    .lock()
                    .map(|runs| runs.contains_key(&handle.run_id))
                    .unwrap_or(false);
                if !active {
                    break;
                }
            }
            let sample = metrics::process_metrics(&handle.pids);
            handle
                .peak_memory
                .fetch_max(sample.memory_bytes, Ordering::Relaxed);
            handle
                .cpu_time
                .fetch_max(sample.cpu_time_ms, Ordering::Relaxed);
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    });
}

fn spawn_timeout(app: AppHandle, handle: RunHandle) {
    let Some(timeout_sec) = handle.timeout_sec.filter(|value| *value > 0) else {
        return;
    };
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_secs(timeout_sec as u64)).await;
        let state = app.state::<AppState>();
        let active = state
            .runs
            .lock()
            .map(|runs| runs.contains_key(&handle.run_id))
            .unwrap_or(false);
        if !active {
            return;
        }
        if let Ok(mut reason) = handle.stop_reason.lock() {
            *reason = Some("timeout".into());
        }
        handle.stopped.store(true, Ordering::SeqCst);
        handle.guard.terminate();
        terminate_process_trees(&handle.pids);
        logging::app_log(
            "WARN",
            &format!("脚本 {} 达到运行超时 {} 秒", handle.script_id, timeout_sec),
        );
    });
}

fn spawn_waiter(app: AppHandle, mut child: tokio::process::Child, handle: RunHandle) {
    tauri::async_runtime::spawn(async move {
        let wait_result = child.wait().await;
        let (exit_code, wait_error) = match wait_result {
            Ok(status) => (status.code(), None),
            Err(error) => (None, Some(format!("等待进程结束失败: {error}"))),
        };

        while handle.guard.active_processes() > 0 {
            tokio::time::sleep(Duration::from_millis(200)).await;
        }

        let reason = handle
            .stop_reason
            .lock()
            .ok()
            .and_then(|value| value.clone());
        let success =
            exit_code.is_some_and(|code| success_code_allowed(&handle.success_codes, code));
        let final_status = if reason.as_deref() == Some("timeout") {
            "failed"
        } else if reason.is_some() {
            "stopped"
        } else if success {
            "exited"
        } else {
            "failed"
        };
        let error = wait_error.or_else(|| {
            if reason.as_deref() == Some("timeout") {
                Some("运行超时".to_string())
            } else if final_status == "failed" {
                Some(format!("进程退出码 {}", exit_code.unwrap_or(-1)))
            } else {
                None
            }
        });
        finish_run(&app, &handle, final_status, exit_code, error);
    });
}

pub fn active_count(app: &AppHandle, script_id: &str) -> usize {
    app.state::<AppState>()
        .runs
        .lock()
        .map(|runs| {
            runs.values()
                .filter(|run| run.script_id == script_id)
                .count()
        })
        .unwrap_or(0)
}

fn force_stop_script(app: &AppHandle, script_id: &str, reason: &str) {
    let handles = app
        .state::<AppState>()
        .runs
        .lock()
        .map(|runs| {
            runs.values()
                .filter(|run| run.script_id == script_id)
                .cloned()
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    for handle in handles {
        if let Ok(mut value) = handle.stop_reason.lock() {
            *value = Some(reason.to_string());
        }
        handle.stopped.store(true, Ordering::SeqCst);
        handle.guard.terminate();
        terminate_process_trees(&handle.pids);
    }
    std::thread::sleep(Duration::from_millis(250));
}

pub fn start(app: &AppHandle, script: Script) -> Result<RunStateEvent, String> {
    let options = StartOptions {
        source: "manual".into(),
        source_id: None,
        timeout_sec: script.timeout_sec,
        success_codes: script.success_codes.clone(),
        overlap_policy: script.instance_policy.clone(),
    };
    start_with(app, script, options)
}

pub fn start_with(
    app: &AppHandle,
    script: Script,
    options: StartOptions,
) -> Result<RunStateEvent, String> {
    preflight(app, &script)?;
    let active = active_count(app, &script.id);
    let max_instances = script.max_instances.max(1) as usize;
    if active >= max_instances {
        match options.overlap_policy.as_str() {
            "skip" => {
                return Err(format!(
                    "{} 已有 {} 个实例运行中，已按 skip 策略跳过",
                    script.name, active
                ));
            }
            "replace" => {
                force_stop_script(app, &script.id, "replace");
            }
            "queue" => {
                let queued_id = format!("queued-{}", Uuid::new_v4());
                let started_at = db::now_ms();
                let event = RunStateEvent {
                    run_id: queued_id.clone(),
                    script_id: script.id.clone(),
                    status: "queued".into(),
                    exit_code: None,
                    error: None,
                    started_at,
                    ended_at: None,
                    source: Some(options.source.clone()),
                    source_id: options.source_id.clone(),
                };
                let _ = app.emit("run-state", event.clone());
                let app_for_queue = app.clone();
                let mut queued_options = options.clone();
                queued_options.overlap_policy = "parallel".into();
                tauri::async_runtime::spawn(async move {
                    loop {
                        if app_for_queue
                            .state::<AppState>()
                            .exiting
                            .load(Ordering::SeqCst)
                        {
                            return;
                        }
                        if active_count(&app_for_queue, &script.id) == 0 {
                            break;
                        }
                        tokio::time::sleep(Duration::from_millis(300)).await;
                    }
                    if let Err(error) = start_with(&app_for_queue, script.clone(), queued_options) {
                        let _ = app_for_queue.emit(
                            "run-state",
                            RunStateEvent {
                                run_id: queued_id,
                                script_id: script.id,
                                status: "failed".into(),
                                exit_code: None,
                                error: Some(error.clone()),
                                started_at,
                                ended_at: Some(db::now_ms()),
                                source: Some("queue".into()),
                                source_id: None,
                            },
                        );
                    }
                });
                return Ok(event);
            }
            "parallel" => {
                return Err(format!(
                    "{} 已达到最大实例数 {}，请调整实例策略",
                    script.name, max_instances
                ));
            }
            _ => {}
        }
    }

    let spec = runtime::find_spec(&script.runtime_id)
        .ok_or_else(|| format!("未知运行时: {}", script.runtime_id))?;
    let exe =
        runtime::resolve_executable(spec, script.interpreter_path.as_deref(), Some(&script.path))?;
    let mut command = build_command(&script, &exe)?;
    let mut child = command.spawn().map_err(|e| {
        logging::app_log("ERROR", &format!("启动 {} 失败: {e}", script.name));
        format!("启动进程失败: {e}")
    })?;
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
            &options.source,
            options.source_id.as_deref(),
            options.timeout_sec,
        ) {
            logging::app_log("ERROR", &format!("写入运行记录失败: {error}"));
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
        pids: vec![pid],
        guard: guard.clone(),
        stopped: stopped.clone(),
        stop_reason: Arc::new(StdMutex::new(None)),
        started_at,
        stop_timeout_sec: script.stop_timeout_sec.max(1),
        stop_mode: script.stop_mode.clone(),
        timeout_sec: options.timeout_sec,
        success_codes: options.success_codes.clone(),
        source: options.source.clone(),
        source_id: options.source_id.clone(),
        ring: Arc::new(LogRing::new(LOG_RING_BYTES)),
        peak_memory: Arc::new(AtomicU64::new(0)),
        cpu_time: Arc::new(AtomicU64::new(0)),
        log_bytes: Arc::new(AtomicU64::new(0)),
    };
    if let Err(error) = state
        .runs
        .lock()
        .map_err(|_| "运行状态锁失败".to_string())
        .map(|mut runs| {
            runs.insert(run_id.clone(), handle.clone());
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
        source: Some(options.source.clone()),
        source_id: options.source_id.clone(),
    };
    let _ = app.emit("run-state", event.clone());
    let _ = state.lifecycle_tx.send(LifecycleEvent {
        run_id: run_id.clone(),
        script_id: script.id.clone(),
        status: "running".into(),
        exit_code: None,
        source: options.source.clone(),
        source_id: options.source_id.clone(),
    });

    let log = Arc::new(AsyncMutex::new(BufWriter::with_capacity(
        64 * 1024,
        log_file,
    )));
    let sink = || OutputSink {
        log: log.clone(),
        app: app.clone(),
        run_id: run_id.clone(),
        script_id: script.id.clone(),
        ring: handle.ring.clone(),
        log_bytes: handle.log_bytes.clone(),
    };
    if let Some(stdout) = child.stdout.take() {
        let sink = sink();
        tauri::async_runtime::spawn(async move {
            pump_output(stdout, "stdout", sink).await;
        });
    }
    if let Some(stderr) = child.stderr.take() {
        let sink = sink();
        tauri::async_runtime::spawn(async move {
            pump_output(stderr, "stderr", sink).await;
        });
    }

    spawn_metrics_sampler(app.clone(), handle.clone());
    spawn_timeout(app.clone(), handle.clone());
    spawn_waiter(app.clone(), child, handle);

    Ok(event)
}

fn stop_handle(app: &AppHandle, handle: &RunHandle, reason: &str, force: bool) {
    handle.stopped.store(true, Ordering::SeqCst);
    if let Ok(mut value) = handle.stop_reason.lock() {
        *value = Some(reason.to_string());
    }
    let force = force || handle.stop_mode == "force";
    if force {
        handle.guard.terminate();
        terminate_process_trees(&handle.pids);
    } else {
        handle
            .guard
            .request_graceful(handle.pid, &handle.runtime_id);
    }
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
            source: Some(handle.source.clone()),
            source_id: handle.source_id.clone(),
        },
    );

    if force {
        return;
    }
    let app_for_timer = app.clone();
    let guard = handle.guard.clone();
    let pids = handle.pids.clone();
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
            terminate_process_trees(&pids);
        }
    });
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
    for handle in &handles {
        stop_handle(app, handle, "user", false);
    }
    Ok(())
}

pub fn stop_all(app: &AppHandle, force: bool) {
    let state = app.state::<AppState>();
    let handles = {
        let runs = state.runs.lock().ok();
        runs.map(|runs| runs.values().cloned().collect::<Vec<_>>())
            .unwrap_or_default()
    };
    let reason = if force { "app_exit" } else { "user" };
    for handle in &handles {
        stop_handle(app, handle, reason, force);
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::{resume_process, success_code_allowed};
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

    #[test]
    fn checks_success_codes() {
        assert!(success_code_allowed("0", 0));
        assert!(!success_code_allowed("0", 1));
        assert!(success_code_allowed("0,2,10-12", 11));
        assert!(success_code_allowed("*", 9));
    }
}
