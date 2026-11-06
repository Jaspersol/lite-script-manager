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
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager};
use tokio::io::{AsyncReadExt, AsyncWriteExt, BufWriter};
use tokio::sync::{Mutex as AsyncMutex, Notify, mpsc};
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

    pub fn request_graceful(&self, pid: u32, pids: &[u32], runtime_id: &str, adopted: bool) {
        if adopted {
            // Adopted processes are not process-group leaders, so CTRL_BREAK never
            // reaches them; only window messages can ask them to close.
            for pid in pids {
                post_wm_close(*pid);
            }
            return;
        }
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
        GetWindowThreadProcessId, PostMessageW, WM_CLOSE,
    };

    let target_pid = lparam as u32;
    let mut window_pid = 0u32;
    unsafe {
        GetWindowThreadProcessId(hwnd, &mut window_pid);
        // Hidden windows count too: an AutoHotkey main window is hidden.
        if window_pid == target_pid {
            PostMessageW(hwnd, WM_CLOSE, 0, 0);
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

/// `FreeConsole`/`AttachConsole` change the console of the whole process, so
/// concurrent stops must not interleave.
#[cfg(windows)]
static CONSOLE_LOCK: StdMutex<()> = StdMutex::new(());

#[cfg(windows)]
fn generate_ctrl_break(pid: u32) {
    use windows_sys::Win32::System::Console::{
        AttachConsole, CTRL_BREAK_EVENT, FreeConsole, GenerateConsoleCtrlEvent,
    };
    let _guard = CONSOLE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
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
            Err("恢复挂起的子进程线程失败".to_string())
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
    use windows_sys::Win32::Foundation::{CloseHandle, STILL_ACTIVE};
    use windows_sys::Win32::System::Threading::{
        GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };

    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle.is_null() {
            return false;
        }
        // A handle held elsewhere keeps an exited process openable, so check the exit code.
        let mut code = 0u32;
        let ok = GetExitCodeProcess(handle, &mut code);
        CloseHandle(handle);
        ok == 0 || code == STILL_ACTIVE as u32
    }
}

#[cfg(not(windows))]
pub fn process_alive(_pid: u32) -> bool {
    false
}

/// True while any recorded process is alive and is still the one we saw start
/// (the creation time guards against PID reuse).
fn run_processes_alive(pids: &[u32], started: &[Option<i64>]) -> bool {
    pids.iter().enumerate().any(|(index, pid)| {
        process_alive(*pid)
            && match started.get(index).copied().flatten() {
                None => true,
                Some(created) => process_create_time_ms(*pid) == Some(created),
            }
    })
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
            .chain(runtime::adopt_executables(spec))
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

/// Clears the adopt-in-progress flag even if the scan unwinds.
#[cfg(windows)]
struct AdoptingReset<'a>(&'a AtomicBool);

#[cfg(windows)]
impl Drop for AdoptingReset<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

#[cfg(windows)]
fn spawn_adopted_monitor(app: AppHandle, handle: RunHandle) {
    tauri::async_runtime::spawn(async move {
        while handle.guard.active_processes() > 0
            || run_processes_alive(&handle.pids, &handle.pid_started)
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

/// Collects the PIDs belonging to a script: an explicit `process_match` wins,
/// otherwise the script path is matched against process names and command lines.
fn matching_pids(
    script: &Script,
    processes: &[(u32, u32, String, String)],
    command_lines: &mut HashMap<u32, String>,
    managed_pids: &HashSet<u32>,
    current_pid: u32,
) -> Vec<u32> {
    let pattern = script
        .process_match
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let path_needle = normalized_match(&script.path);
    if pattern.is_none() && path_needle.is_empty() {
        return Vec::new();
    }

    let mut pids = Vec::new();
    for (pid, _parent, exe, normalized_exe) in processes {
        if *pid == current_pid || managed_pids.contains(pid) {
            continue;
        }
        let needle = match pattern {
            Some(value) => normalized_match(value),
            None => path_needle.clone(),
        };
        if needle.is_empty() {
            continue;
        }
        if pattern.is_none() && !executable_allowed(script, exe) {
            continue;
        }
        if normalized_exe.contains(&needle) {
            pids.push(*pid);
            continue;
        }
        let command_line = command_lines
            .entry(*pid)
            .or_insert_with(|| normalized_match(&process_command_line(*pid).unwrap_or_default()));
        if command_line.contains(&needle) {
            pids.push(*pid);
        }
    }
    pids
}

#[cfg(windows)]
fn attach_running(app: &AppHandle, script: &Script) -> Option<RunStateEvent> {
    let state = app.state::<AppState>();
    let processes = snapshot_processes()
        .into_iter()
        .map(|(pid, parent, exe)| {
            let normalized = normalized_match(&exe);
            (pid, parent, exe, normalized)
        })
        .collect::<Vec<_>>();
    if processes.is_empty() {
        return None;
    }
    let managed_pids = state
        .runs
        .lock()
        .map(|runs| {
            runs.values()
                .flat_map(|run| run.pids.iter().copied())
                .collect::<HashSet<_>>()
        })
        .unwrap_or_default();
    let mut command_lines = HashMap::new();
    let pids = matching_pids(
        script,
        &processes,
        &mut command_lines,
        &managed_pids,
        std::process::id(),
    );
    if pids.is_empty() {
        return None;
    }
    let guard = match JobHandle::attach_many(&pids) {
        Ok(guard) => Arc::new(guard),
        Err(error) => {
            logging::app_log("WARN", &format!("接管 {} 失败: {error}", script.name));
            return None;
        }
    };
    let run_id = Uuid::new_v4().to_string();
    let started_at = db::now_ms();
    {
        let conn = state.db.lock().ok()?;
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
            return None;
        }
    }
    let pid_started = pids
        .iter()
        .map(|pid| process_create_time_ms(*pid))
        .collect::<Vec<_>>();
    let handle = RunHandle {
        run_id: run_id.clone(),
        script_id: script.id.clone(),
        runtime_id: script.runtime_id.clone(),
        pid: pids[0],
        pids: pids.clone(),
        pid_started,
        guard: guard.clone(),
        stopped: Arc::new(AtomicBool::new(false)),
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
        finished: Arc::new(AtomicBool::new(false)),
        done: Arc::new(Notify::new()),
    };
    if let Ok(mut runs) = state.runs.lock() {
        runs.insert(run_id.clone(), handle.clone());
    } else {
        guard.terminate();
        return None;
    }
    let event = RunStateEvent {
        run_id: run_id.clone(),
        script_id: script.id.clone(),
        status: "running".into(),
        exit_code: None,
        error: None,
        started_at,
        ended_at: None,
        source: Some("adopted".into()),
        source_id: None,
    };
    let _ = app.emit("run-state", event.clone());
    let _ = state.lifecycle_tx.send(LifecycleEvent {
        run_id: run_id.clone(),
        script_id: script.id.clone(),
        status: "running".into(),
        exit_code: None,
        source: "adopted".into(),
        source_id: None,
    });
    spawn_adopted_monitor(app.clone(), handle);
    logging::app_log(
        "INFO",
        &format!("已接管正在运行的 {}（PID {:?}）", script.name, pids),
    );
    Some(event)
}

#[cfg(not(windows))]
fn attach_running(_app: &AppHandle, _script: &Script) -> Option<RunStateEvent> {
    None
}

#[cfg(windows)]
pub fn adopt_external(app: &AppHandle) -> Result<usize, String> {
    let state = app.state::<AppState>();
    if state.adopting.swap(true, Ordering::SeqCst) {
        return Ok(0);
    }
    let reset = AdoptingReset(&state.adopting);

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
            let pids = runs
                .values()
                .flat_map(|run| run.pids.iter().copied())
                .collect::<HashSet<_>>();
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
            let lock = script_lock(app, &script.id);
            let _start_guard = lock.lock().unwrap_or_else(|e| e.into_inner());
            if active_count(app, &script.id) > 0 {
                continue;
            }

            let pids = matching_pids(
                &script,
                &processes,
                &mut command_lines,
                &managed_pids,
                current_pid,
            );
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
            let pid_started = pids
                .iter()
                .map(|pid| process_create_time_ms(*pid))
                .collect::<Vec<_>>();
            let handle = RunHandle {
                run_id: run_id.clone(),
                script_id: script.id.clone(),
                runtime_id: script.runtime_id.clone(),
                pid: pids[0],
                pids: pids.clone(),
                pid_started,
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
                finished: Arc::new(AtomicBool::new(false)),
                done: Arc::new(Notify::new()),
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
            let _ = state.lifecycle_tx.send(LifecycleEvent {
                run_id: run_id.clone(),
                script_id: script.id.clone(),
                status: "running".into(),
                exit_code: None,
                source: "adopted".into(),
                source_id: None,
            });
            spawn_adopted_monitor(app.clone(), handle);
            adopted += 1;
        }
        Ok(adopted)
    })();

    drop(reset);
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

    pub fn request_graceful(&self, _pid: u32, _pids: &[u32], _runtime_id: &str, _adopted: bool) {
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
    /// Creation time per entry of `pids`, to tell a reused PID from the original process.
    pub pid_started: Vec<Option<i64>>,
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
    /// Set by the first `finish_run`; later calls for the same run are no-ops.
    pub finished: Arc<AtomicBool>,
    /// Signalled when the run is finalized, so per-run timers can exit early.
    pub done: Arc<Notify>,
}

/// A start request waiting for a free instance slot (`queue` overlap policy).
#[derive(Clone)]
pub struct QueuedStart {
    pub queued_id: String,
    pub script: Script,
    pub options: StartOptions,
    pub started_at: i64,
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

/// Parses `0`, `10-12`, `0xC0000005` or `0xC0000005-0xC0000010`.
fn parse_exit_code(token: &str) -> Option<u32> {
    let token = token.trim();
    if let Some(hex) = token
        .strip_prefix("0x")
        .or_else(|| token.strip_prefix("0X"))
    {
        return u32::from_str_radix(hex, 16).ok();
    }
    token.parse::<u32>().ok()
}

/// Rejects malformed `success_codes` before it is stored, so the run status is
/// never computed from an expression the matcher cannot read.
pub(crate) fn validate_success_codes(expression: &str) -> Result<(), String> {
    let expression = expression.trim();
    if expression.is_empty() || expression == "*" {
        return Ok(());
    }
    for part in expression.split(',') {
        let part = part.trim();
        let valid = if let Some((start, end)) = part.split_once('-') {
            match (parse_exit_code(start), parse_exit_code(end)) {
                (Some(start), Some(end)) => start <= end,
                _ => false,
            }
        } else {
            parse_exit_code(part).is_some()
        };
        if !valid {
            return Err(format!("成功退出码格式无效: {part}"));
        }
    }
    Ok(())
}

fn success_code_allowed(expression: &str, code: u32) -> bool {
    let expression = expression.trim();
    if expression.is_empty() || expression == "*" {
        return true;
    }
    for part in expression.split(',') {
        let part = part.trim();
        if let Some((start, end)) = part.split_once('-') {
            if let (Some(start), Some(end)) = (parse_exit_code(start), parse_exit_code(end))
                && code >= start
                && code <= end
            {
                return true;
            }
        } else if parse_exit_code(part) == Some(code) {
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

/// Strips the `\\?\` verbatim prefix, which `cmd.exe` and `powershell -File`
/// cannot open.
pub fn plain_path(path: &str) -> String {
    if let Some(rest) = path.strip_prefix(r"\\?\UNC\") {
        return format!(r"\\{rest}");
    }
    if let Some(rest) = path.strip_prefix(r"\\?\") {
        return rest.to_string();
    }
    path.to_string()
}

fn build_command(script: &Script, exe: &PathBuf) -> Result<tokio::process::Command, String> {
    let spec = runtime::find_spec(&script.runtime_id)
        .ok_or_else(|| format!("未知运行时: {}", script.runtime_id))?;
    let plain = plain_path(&script.path);
    let script_path = if spec.id == "shell" {
        plain.replace('\\', "/")
    } else {
        plain
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

const OUTPUT_READ_CHUNK: usize = 8 * 1024;
/// A line without a newline is emitted in pieces beyond this, so a process that
/// never prints `\n` cannot grow the buffer without bound.
const OUTPUT_PENDING_LIMIT: usize = 64 * 1024;

/// Reads a pipe on a dedicated task and forwards complete lines. The read
/// buffer lives across reads, so no byte is lost when the pump flushes early.
fn spawn_line_reader<R>(mut reader: R, sender: mpsc::Sender<Vec<u8>>)
where
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
{
    tauri::async_runtime::spawn(async move {
        let mut buffer = [0u8; OUTPUT_READ_CHUNK];
        let mut pending: Vec<u8> = Vec::new();
        loop {
            let read = match reader.read(&mut buffer).await {
                Ok(0) | Err(_) => break,
                Ok(read) => read,
            };
            pending.extend_from_slice(&buffer[..read]);
            while let Some(index) = pending.iter().position(|byte| *byte == b'\n') {
                let mut line: Vec<u8> = pending.drain(..=index).collect();
                line.pop();
                while line.last().is_some_and(|byte| *byte == b'\r') {
                    line.pop();
                }
                if sender.send(line).await.is_err() {
                    return;
                }
            }
            while pending.len() > OUTPUT_PENDING_LIMIT {
                let chunk: Vec<u8> = pending.drain(..OUTPUT_PENDING_LIMIT).collect();
                if sender.send(chunk).await.is_err() {
                    return;
                }
            }
        }
        while pending.last().is_some_and(|byte| *byte == b'\r') {
            pending.pop();
        }
        if !pending.is_empty() {
            let _ = sender.send(pending).await;
        }
    });
}

async fn pump_output<R>(reader: R, stream: &'static str, sink: OutputSink)
where
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
{
    let OutputSink {
        log,
        app,
        run_id,
        script_id,
        ring,
        log_bytes,
    } = sink;
    let (sender, mut receiver) = mpsc::channel::<Vec<u8>>(256);
    spawn_line_reader(reader, sender);
    let mut ticker = tokio::time::interval(Duration::from_millis(100));
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut batch: Vec<LogLine> = Vec::new();

    loop {
        tokio::select! {
            _ = ticker.tick() => {
                let _ = log.lock().await.flush().await;
                if app.state::<AppState>().log_streaming.load(Ordering::Relaxed) {
                    flush_log_batch(&app, &run_id, &script_id, &mut batch);
                } else {
                    batch.clear();
                }
            }
            bytes = receiver.recv() => {
                let Some(bytes) = bytes else {
                    if app.state::<AppState>().log_streaming.load(Ordering::Relaxed) {
                        flush_log_batch(&app, &run_id, &script_id, &mut batch);
                    }
                    break;
                };
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
        }
    }
    let _ = log.lock().await.flush().await;
}

fn finish_run(
    app: &AppHandle,
    run: &RunHandle,
    status: &str,
    exit_code: Option<u32>,
    error: Option<String>,
) {
    if run.finished.swap(true, Ordering::SeqCst) {
        return;
    }
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
    run.done.notify_one();
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
    dispatch_queue_async(app, &run.script_id);
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

fn spawn_timeout(app: AppHandle, handle: &RunHandle) {
    let Some(timeout_sec) = handle.timeout_sec.filter(|value| *value > 0) else {
        return;
    };
    // Hold only the id and the notifier while sleeping: the full handle owns the
    // log ring and the Job handle, which must be released when the run ends.
    let run_id = handle.run_id.clone();
    let done = handle.done.clone();
    tauri::async_runtime::spawn(async move {
        tokio::select! {
            _ = done.notified() => return,
            _ = tokio::time::sleep(Duration::from_secs(timeout_sec as u64)) => {}
        }
        let state = app.state::<AppState>();
        let handle = state
            .runs
            .lock()
            .ok()
            .and_then(|runs| runs.get(&run_id).cloned());
        let Some(handle) = handle else {
            return;
        };
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

fn spawn_waiter(
    app: AppHandle,
    mut child: tokio::process::Child,
    handle: RunHandle,
    pumps: Vec<tauri::async_runtime::JoinHandle<()>>,
) {
    tauri::async_runtime::spawn(async move {
        let wait_result = child.wait().await;
        let (exit_code, wait_error) = match wait_result {
            // Windows exit codes above i32::MAX arrive as a negative i32; the raw
            // DWORD is kept by reinterpreting the bits as unsigned.
            Ok(status) => (status.code().map(|code| code as u32), None),
            Err(error) => (None, Some(format!("等待进程结束失败: {error}"))),
        };

        while handle.guard.active_processes() > 0 {
            tokio::time::sleep(Duration::from_millis(200)).await;
        }

        // Output pumps end once every process holding the pipes is gone. Wait for
        // them so the log tail and `log_bytes` are complete, but bound the wait.
        let _ = tokio::time::timeout(Duration::from_secs(3), async {
            for pump in pumps {
                let _ = pump.await;
            }
        })
        .await;

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
                Some(match exit_code {
                    Some(code) => format!("进程退出码 {code}"),
                    None => "进程因未知原因退出".to_string(),
                })
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

/// Grace period so a freshly spawned child is not mistaken for a dead one.
const STALE_RUN_MIN_AGE: Duration = Duration::from_secs(5);

/// Finalizes runs whose process tree is already gone. A waiter that never ran
/// leaves the handle in `runs`, which keeps occupying an instance slot and
/// rejects every later start as "already running".
pub fn sweep_dead_runs(app: &AppHandle) {
    let state = app.state::<AppState>();
    let dead = {
        let Ok(runs) = state.runs.lock() else {
            return;
        };
        let now = db::now_ms();
        runs.values()
            .filter(|run| {
                now.saturating_sub(run.started_at) >= STALE_RUN_MIN_AGE.as_millis() as i64
            })
            .filter(|run| run.guard.active_processes() == 0)
            .filter(|run| !run_processes_alive(&run.pids, &run.pid_started))
            .cloned()
            .collect::<Vec<_>>()
    };
    for handle in dead {
        logging::app_log(
            "WARN",
            &format!("回收已结束但未收尾的运行 {}", handle.run_id),
        );
        finish_run(app, &handle, "exited", None, None);
    }
}

pub(crate) fn script_lock(app: &AppHandle, script_id: &str) -> Arc<StdMutex<()>> {
    let state = app.state::<AppState>();
    let mut locks = state.start_locks.lock().unwrap_or_else(|e| e.into_inner());
    locks.entry(script_id.to_string()).or_default().clone()
}

fn script_handles(app: &AppHandle, script_id: &str) -> Vec<RunHandle> {
    app.state::<AppState>()
        .runs
        .lock()
        .map(|runs| {
            runs.values()
                .filter(|run| run.script_id == script_id)
                .cloned()
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

/// Stops every instance of the script per its `stop_mode` and waits until none
/// is left. `stop_handle` force-kills after `stop_timeout_sec`; the extra margin
/// lets the waiter observe the exit.
fn stop_and_wait(app: &AppHandle, script_id: &str, reason: &str) {
    let handles = script_handles(app, script_id);
    if handles.is_empty() {
        return;
    }
    let timeout = handles
        .iter()
        .map(|handle| handle.stop_timeout_sec.max(1) as u64)
        .max()
        .unwrap_or(1);
    for handle in &handles {
        stop_handle(app, handle, reason, false);
    }
    let deadline = Instant::now() + Duration::from_secs(timeout + 3);
    while active_count(app, script_id) > 0 && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Stops every instance of the script before it is deleted. Graceful stop is
/// tried first; whatever survives the wait is killed so deleting the script can
/// never leave an orphan process that the UI no longer tracks.
pub(crate) fn stop_for_delete(app: &AppHandle, script_id: &str) {
    // Hold the script lock so a queue dispatch or start that is already in
    // flight finishes before the queue is drained and the processes are killed.
    let lock = script_lock(app, script_id);
    let _guard = lock.lock().unwrap_or_else(|e| e.into_inner());
    // Queued starts would otherwise be dispatched after the row is gone.
    let _ = take_queue(app, Some(script_id));
    stop_and_wait(app, script_id, "script_deleted");
    for handle in script_handles(app, script_id) {
        handle.stopped.store(true, Ordering::SeqCst);
        handle.guard.terminate();
        terminate_process_trees(&handle.pids);
    }
    let deadline = Instant::now() + Duration::from_secs(3);
    while active_count(app, script_id) > 0 && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn take_queue(app: &AppHandle, script_id: Option<&str>) -> Vec<QueuedStart> {
    let state = app.state::<AppState>();
    let Ok(mut queues) = state.queues.lock() else {
        return Vec::new();
    };
    match script_id {
        Some(id) => queues
            .remove(id)
            .map(|queue| queue.into_iter().collect())
            .unwrap_or_default(),
        None => queues.drain().flat_map(|(_, queue)| queue).collect(),
    }
}

/// Reports cancelled queue entries. When the script still has a running
/// instance, that instance's own events carry the UI state, so the placeholder
/// is not announced.
fn settle_cancelled(app: &AppHandle, items: Vec<QueuedStart>) {
    if items.is_empty() {
        return;
    }
    let state = app.state::<AppState>();
    if let Ok(conn) = state.db.lock() {
        for item in &items {
            let _ = db::mark_schedule_result_by_run(&conn, &item.queued_id, "stopped", None);
        }
    }
    for item in items {
        if active_count(app, &item.script.id) == 0 {
            let _ = app.emit(
                "run-state",
                RunStateEvent {
                    run_id: item.queued_id,
                    script_id: item.script.id,
                    status: "stopped".into(),
                    exit_code: None,
                    error: None,
                    started_at: item.started_at,
                    ended_at: Some(db::now_ms()),
                    source: Some(item.options.source),
                    source_id: item.options.source_id,
                },
            );
        }
    }
    let _ = app.emit("schedules-changed", ());
}

/// Records the real run id for a start that was first reported as `queued-*`.
fn bind_queued(app: &AppHandle, queued_id: &str, run_id: &str) {
    let state = app.state::<AppState>();
    if let Ok(conn) = state.db.lock() {
        let _ = db::rebind_schedule_run(&conn, queued_id, run_id);
    }
    if let Ok(mut alias) = state.queue_alias.lock() {
        if alias.len() >= 256 {
            alias.clear();
        }
        alias.insert(queued_id.to_string(), run_id.to_string());
    }
    let _ = app.emit("schedules-changed", ());
}

/// The real run id a `queued-*` placeholder was started as, if it has started.
pub fn queued_alias(app: &AppHandle, queued_id: &str) -> Option<String> {
    app.state::<AppState>()
        .queue_alias
        .lock()
        .ok()
        .and_then(|alias| alias.get(queued_id).cloned())
}

fn dispatch_queue_async(app: &AppHandle, script_id: &str) {
    let pending = app
        .state::<AppState>()
        .queues
        .lock()
        .map(|queues| queues.contains_key(script_id))
        .unwrap_or(false);
    if !pending {
        return;
    }
    let app = app.clone();
    let script_id = script_id.to_string();
    tauri::async_runtime::spawn_blocking(move || dispatch_queue(&app, &script_id));
}

/// Starts queued entries in FIFO order while the script has free instance slots.
fn dispatch_queue(app: &AppHandle, script_id: &str) {
    let state = app.state::<AppState>();
    let lock = script_lock(app, script_id);
    let _guard = lock.lock().unwrap_or_else(|e| e.into_inner());
    loop {
        if state.exiting.load(Ordering::SeqCst) {
            return;
        }
        sweep_dead_runs(app);
        let active = active_count(app, script_id);
        let item = {
            let Ok(mut queues) = state.queues.lock() else {
                return;
            };
            let ready = queues
                .get(script_id)
                .and_then(|queue| queue.front())
                .is_some_and(|front| active < front.script.max_instances.max(1) as usize);
            if !ready {
                if queues.get(script_id).is_some_and(|queue| queue.is_empty()) {
                    queues.remove(script_id);
                }
                return;
            }
            let Some(queue) = queues.get_mut(script_id) else {
                return;
            };
            let item = queue.pop_front();
            if queue.is_empty() {
                queues.remove(script_id);
            }
            item
        };
        let Some(item) = item else {
            return;
        };
        let result = preflight(app, &item.script).and_then(|_| {
            start_locked(
                app,
                item.script.clone(),
                item.options.clone(),
                false,
                Some(item.queued_id.clone()),
            )
        });
        if let Err(error) = result {
            logging::app_log(
                "WARN",
                &format!("排队启动 {} 失败: {error}", item.script.name),
            );
            if let Ok(conn) = state.db.lock() {
                let _ =
                    db::mark_schedule_result_by_run(&conn, &item.queued_id, "failed", Some(&error));
            }
            let _ = app.emit(
                "run-state",
                RunStateEvent {
                    run_id: item.queued_id,
                    script_id: item.script.id,
                    status: "failed".into(),
                    exit_code: None,
                    error: Some(error),
                    started_at: item.started_at,
                    ended_at: Some(db::now_ms()),
                    source: Some(item.options.source),
                    source_id: item.options.source_id,
                },
            );
            let _ = app.emit("schedules-changed", ());
        }
    }
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
    start_inner(app, script, options, false)
}

/// Stops every instance of the script per its `stop_mode`, waits for them to
/// exit, then starts a fresh one.
pub fn restart_with(
    app: &AppHandle,
    script: Script,
    options: StartOptions,
) -> Result<RunStateEvent, String> {
    start_inner(app, script, options, true)
}

fn start_inner(
    app: &AppHandle,
    script: Script,
    options: StartOptions,
    restart: bool,
) -> Result<RunStateEvent, String> {
    preflight(app, &script)?;
    sweep_dead_runs(app);
    // The instance-count check, overlap policy and `runs.insert` must not
    // interleave with another start of the same script.
    let lock = script_lock(app, &script.id);
    let _guard = lock.lock().unwrap_or_else(|e| e.into_inner());
    start_locked(app, script, options, restart, None)
}

fn enqueue(app: &AppHandle, script: Script, options: StartOptions) -> RunStateEvent {
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
    let script_id = script.id.clone();
    if let Ok(mut queues) = app.state::<AppState>().queues.lock() {
        queues
            .entry(script_id.clone())
            .or_default()
            .push_back(QueuedStart {
                queued_id,
                script,
                options,
                started_at,
            });
    }
    let _ = app.emit("run-state", event.clone());
    // An instance may have finished between the count check and the push;
    // dispatching again keeps the entry from being stranded.
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || dispatch_queue(&app, &script_id));
    event
}

/// Caller holds the script lock.
fn start_locked(
    app: &AppHandle,
    script: Script,
    options: StartOptions,
    restart: bool,
    queued_id: Option<String>,
) -> Result<RunStateEvent, String> {
    if restart {
        stop_and_wait(app, &script.id, "replace");
    }
    // Adopt an instance still running from a previous session instead of
    // spawning a duplicate or refusing with "already running".
    if active_count(app, &script.id) == 0
        && script.max_instances <= 1
        && let Some(event) = attach_running(app, &script)
    {
        if let Some(queued_id) = &queued_id {
            bind_queued(app, queued_id, &event.run_id);
        }
        return Ok(event);
    }
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
            "replace" => stop_and_wait(app, &script.id, "replace"),
            "queue" => return Ok(enqueue(app, script, options)),
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
    let pid = child
        .id()
        .ok_or_else(|| "获取子进程 PID 失败".to_string())?;

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

    if let Some(queued_id) = &queued_id {
        bind_queued(app, queued_id, &run_id);
    }
    // Resume only after the log and DB row exist, so a failure above never
    // leaves a script that already started executing.
    if let Err(error) = resume_process(pid) {
        guard.terminate();
        let _ = child.start_kill();
        if let Ok(conn) = state.db.lock() {
            let _ = db::finish_run(
                &conn,
                &run_id,
                "failed",
                None,
                Some(&error),
                None,
                None,
                None,
                None,
            );
        }
        return Err(error);
    }
    let pid_started = vec![process_create_time_ms(pid)];

    let stopped = Arc::new(AtomicBool::new(false));
    let handle = RunHandle {
        run_id: run_id.clone(),
        script_id: script.id.clone(),
        runtime_id: script.runtime_id.clone(),
        pid,
        pids: vec![pid],
        pid_started,
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
        finished: Arc::new(AtomicBool::new(false)),
        done: Arc::new(Notify::new()),
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
    let mut pumps = Vec::new();
    if let Some(stdout) = child.stdout.take() {
        let sink = sink();
        pumps.push(tauri::async_runtime::spawn(async move {
            pump_output(stdout, "stdout", sink).await;
        }));
    }
    if let Some(stderr) = child.stderr.take() {
        let sink = sink();
        pumps.push(tauri::async_runtime::spawn(async move {
            pump_output(stderr, "stderr", sink).await;
        }));
    }

    spawn_metrics_sampler(app.clone(), handle.clone());
    spawn_timeout(app.clone(), &handle);
    spawn_waiter(app.clone(), child, handle, pumps);

    Ok(event)
}

fn stop_handle(app: &AppHandle, handle: &RunHandle, reason: &str, force: bool) {
    handle.stopped.store(true, Ordering::SeqCst);
    if let Ok(mut value) = handle.stop_reason.lock() {
        *value = Some(reason.to_string());
    }
    // Announce `stopping` before signalling: a process that dies at once lets
    // the waiter emit `stopped`, and that must stay the last event the UI sees.
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
    let force = force || handle.stop_mode == "force";
    if force {
        handle.guard.terminate();
        terminate_process_trees(&handle.pids);
        return;
    }
    handle.guard.request_graceful(
        handle.pid,
        &handle.pids,
        &handle.runtime_id,
        handle.source == "adopted",
    );
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
    stop_with_reason(app, script_id, "user")
}

/// Cancels pending queued starts of the script, then stops its running
/// instances. `reason` is stored on the run (`user` disables loop schedules).
pub fn stop_with_reason(app: &AppHandle, script_id: &str, reason: &str) -> Result<(), String> {
    // Drop queued starts first so a finishing instance cannot launch them.
    let cancelled = take_queue(app, Some(script_id));
    let had_queue = !cancelled.is_empty();
    settle_cancelled(app, cancelled);
    match stop_instances(app, script_id, reason) {
        Err(_) if had_queue => Ok(()),
        other => other,
    }
}

fn stop_instances(app: &AppHandle, script_id: &str, reason: &str) -> Result<(), String> {
    let state = app.state::<AppState>();
    let mut handles = script_handles(app, script_id);
    if handles.is_empty() {
        // Adopt a process still running from an earlier session so the stop
        // request targets a real handle.
        let script = {
            let conn = state.db.lock().map_err(|_| "数据库锁失败".to_string())?;
            db::get_script(&conn, script_id)?
        }
        .ok_or_else(|| "脚本不存在".to_string())?;
        let attached = {
            let lock = script_lock(app, script_id);
            let _guard = lock.lock().unwrap_or_else(|e| e.into_inner());
            active_count(app, script_id) > 0 || attach_running(app, &script).is_some()
        };
        if !attached {
            return Err("脚本当前没有运行中的实例".to_string());
        }
        handles = script_handles(app, script_id);
        if handles.is_empty() {
            return Err("脚本当前没有运行中的实例".to_string());
        }
    }
    for handle in &handles {
        stop_handle(app, handle, reason, false);
    }
    Ok(())
}

pub fn stop_all(app: &AppHandle, force: bool) {
    let state = app.state::<AppState>();
    settle_cancelled(app, take_queue(app, None));
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

#[cfg(test)]
mod path_tests {
    use super::plain_path;

    #[test]
    fn strips_verbatim_drive_prefix() {
        assert_eq!(plain_path(r"\\?\C:\a\b.vbs"), r"C:\a\b.vbs");
    }

    #[test]
    fn strips_verbatim_unc_prefix() {
        assert_eq!(
            plain_path(r"\\?\UNC\server\share\a.bat"),
            r"\\server\share\a.bat"
        );
    }

    #[test]
    fn keeps_ordinary_paths_untouched() {
        assert_eq!(plain_path(r"C:\a\b.vbs"), r"C:\a\b.vbs");
        assert_eq!(plain_path(r"\\server\share\a.bat"), r"\\server\share\a.bat");
    }

    /// Regression: `cmd.exe` rejects a verbatim path.
    #[cfg(windows)]
    #[test]
    fn batch_runs_from_a_plain_path() {
        use std::os::windows::process::CommandExt;
        let dir = std::env::temp_dir();
        let script = dir.join(format!("lsm-batch-{}.bat", uuid::Uuid::new_v4()));
        std::fs::write(&script, "@echo off\r\necho ok-plain\r\n").expect("write batch");

        let verbatim = format!("{}{}", r"\\?\", script.display());
        let plain = plain_path(&verbatim);
        let mut command = std::process::Command::new(
            std::env::var("COMSPEC").unwrap_or_else(|_| r"C:\Windows\System32\cmd.exe".into()),
        );
        command
            .args(["/d", "/s", "/c", "call", &plain])
            .creation_flags(0x08000000);
        let output = command.output().expect("run batch");
        let stdout = String::from_utf8_lossy(&output.stdout);
        let _ = std::fs::remove_file(&script);
        assert!(output.status.success(), "batch failed with plain path");
        assert!(stdout.contains("ok-plain"));
    }
}

#[cfg(test)]
mod tests {
    use super::{spawn_line_reader, success_code_allowed, validate_success_codes};
    use std::time::Duration;
    use tokio::io::AsyncWriteExt;
    use tokio::sync::mpsc;

    #[tokio::test]
    async fn keeps_partial_lines_across_reads() {
        let (mut writer, reader) = tokio::io::duplex(64);
        let (sender, mut receiver) = mpsc::channel(8);
        spawn_line_reader(reader, sender);
        writer.write_all(b"hel").await.expect("write prefix");
        tokio::time::sleep(Duration::from_millis(20)).await;
        writer.write_all(b"lo\nwor").await.expect("write middle");
        tokio::time::sleep(Duration::from_millis(20)).await;
        writer.write_all(b"ld\r\n").await.expect("write suffix");
        drop(writer);
        assert_eq!(receiver.recv().await.expect("first line"), b"hello");
        assert_eq!(receiver.recv().await.expect("second line"), b"world");
        assert!(receiver.recv().await.is_none(), "channel closes at EOF");
    }

    #[tokio::test]
    async fn flushes_the_partial_line_at_eof() {
        let (mut writer, reader) = tokio::io::duplex(64);
        let (sender, mut receiver) = mpsc::channel(8);
        spawn_line_reader(reader, sender);
        writer.write_all(b"tail\r").await.expect("write tail");
        drop(writer);
        assert_eq!(receiver.recv().await.expect("tail line"), b"tail");
        assert!(receiver.recv().await.is_none(), "channel closes at EOF");
    }

    #[test]
    fn checks_success_codes() {
        assert!(success_code_allowed("0", 0));
        assert!(!success_code_allowed("0", 1));
        assert!(success_code_allowed("0,2,10-12", 11));
        assert!(success_code_allowed("*", 9));
    }

    #[test]
    fn accepts_unsigned_windows_exit_codes() {
        assert!(success_code_allowed("3221225477", 3_221_225_477));
        assert!(success_code_allowed("0xC0000005", 3_221_225_477));
        assert!(success_code_allowed("0xC0000000-0xC0000010", 0xC000_0005));
        assert!(!success_code_allowed("0xC0000005", 1));
        assert!(validate_success_codes("0,2,10-12").is_ok());
        assert!(validate_success_codes("0xC0000005").is_ok());
        assert!(validate_success_codes("").is_ok());
        assert!(validate_success_codes("*").is_ok());
        assert!(validate_success_codes("0,abc").is_err());
        assert!(validate_success_codes("4294967296").is_err());
        assert!(validate_success_codes("-1").is_err());
        assert!(validate_success_codes("10-2").is_err());
    }
}

#[cfg(all(test, windows))]
mod windows_tests {
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
