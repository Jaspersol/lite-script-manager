#[derive(Debug, Clone, Copy, Default)]
pub struct ProcessMetrics {
    pub memory_bytes: u64,
    pub cpu_time_ms: u64,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct SystemMetrics {
    pub cpu_percent: f64,
    pub memory_percent: f64,
    pub memory_used_bytes: u64,
    pub process_count: u32,
    pub disk_free_bytes: u64,
}

#[cfg(windows)]
pub fn process_metrics(pids: &[u32]) -> ProcessMetrics {
    use std::mem::{size_of, zeroed};
    use windows_sys::Win32::Foundation::{CloseHandle, FILETIME};
    use windows_sys::Win32::System::ProcessStatus::{
        GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS,
    };
    use windows_sys::Win32::System::Threading::{
        GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };

    let mut metrics = ProcessMetrics::default();
    for pid in pids {
        unsafe {
            let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, *pid);
            if process.is_null() {
                continue;
            }
            let mut counters: PROCESS_MEMORY_COUNTERS = zeroed();
            counters.cb = size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
            if GetProcessMemoryInfo(
                process,
                &mut counters,
                size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
            ) != 0
            {
                metrics.memory_bytes = metrics
                    .memory_bytes
                    .saturating_add(counters.WorkingSetSize as u64);
            }
            let (mut creation, mut exit, mut kernel, mut user): (
                FILETIME,
                FILETIME,
                FILETIME,
                FILETIME,
            ) = (zeroed(), zeroed(), zeroed(), zeroed());
            if GetProcessTimes(process, &mut creation, &mut exit, &mut kernel, &mut user) != 0 {
                let kernel = ((kernel.dwHighDateTime as u64) << 32) | kernel.dwLowDateTime as u64;
                let user = ((user.dwHighDateTime as u64) << 32) | user.dwLowDateTime as u64;
                metrics.cpu_time_ms = metrics.cpu_time_ms.saturating_add((kernel + user) / 10_000);
            }
            CloseHandle(process);
        }
    }
    metrics
}

#[cfg(not(windows))]
pub fn process_metrics(_pids: &[u32]) -> ProcessMetrics {
    ProcessMetrics::default()
}

#[derive(Default)]
pub struct CpuSampler {
    #[cfg(windows)]
    idle: u64,
    #[cfg(windows)]
    total: u64,
}

#[cfg(windows)]
fn system_times() -> Option<(u64, u64)> {
    use std::mem::zeroed;
    use windows_sys::Win32::Foundation::FILETIME;
    use windows_sys::Win32::System::Threading::GetSystemTimes;
    unsafe {
        let (mut idle, mut kernel, mut user): (FILETIME, FILETIME, FILETIME) =
            (zeroed(), zeroed(), zeroed());
        if GetSystemTimes(&mut idle, &mut kernel, &mut user) == 0 {
            return None;
        }
        let to_u64 =
            |value: FILETIME| ((value.dwHighDateTime as u64) << 32) | value.dwLowDateTime as u64;
        let idle = to_u64(idle);
        let total = to_u64(kernel) + to_u64(user);
        Some((idle, total))
    }
}

impl CpuSampler {
    pub fn sample(&mut self) -> f64 {
        #[cfg(windows)]
        {
            let Some((idle, total)) = system_times() else {
                return 0.0;
            };
            let (idle_delta, total_delta) = (
                idle.saturating_sub(self.idle),
                total.saturating_sub(self.total),
            );
            self.idle = idle;
            self.total = total;
            if total_delta == 0 {
                return 0.0;
            }
            ((total_delta.saturating_sub(idle_delta)) as f64 / total_delta as f64) * 100.0
        }
        #[cfg(not(windows))]
        {
            0.0
        }
    }
}

#[cfg(windows)]
pub fn system_metrics(cpu: &mut CpuSampler) -> SystemMetrics {
    use std::mem::zeroed;
    use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
    use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};

    let mut metrics = SystemMetrics {
        cpu_percent: cpu.sample(),
        ..Default::default()
    };
    unsafe {
        let mut memory: MEMORYSTATUSEX = zeroed();
        memory.dwLength = std::mem::size_of::<MEMORYSTATUSEX>() as u32;
        if GlobalMemoryStatusEx(&mut memory) != 0 {
            metrics.memory_percent = memory.dwMemoryLoad as f64;
            metrics.memory_used_bytes = memory.ullTotalPhys.saturating_sub(memory.ullAvailPhys);
        }
        let mut free = 0u64;
        let mut total = 0u64;
        let root: Vec<u16> = "C:\\\0".encode_utf16().collect();
        if GetDiskFreeSpaceExW(root.as_ptr(), &mut free, &mut total, std::ptr::null_mut()) != 0 {
            metrics.disk_free_bytes = free;
        }
    }
    metrics.process_count = process_count();
    metrics
}

#[cfg(not(windows))]
pub fn system_metrics(cpu: &mut CpuSampler) -> SystemMetrics {
    SystemMetrics {
        cpu_percent: cpu.sample(),
        ..Default::default()
    }
}

#[cfg(windows)]
pub fn process_count() -> u32 {
    use std::mem::{size_of, zeroed};
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
        TH32CS_SNAPPROCESS,
    };
    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snapshot == INVALID_HANDLE_VALUE {
            return 0;
        }
        let mut entry: PROCESSENTRY32W = zeroed();
        entry.dwSize = size_of::<PROCESSENTRY32W>() as u32;
        let mut count = 0;
        if Process32FirstW(snapshot, &mut entry) != 0 {
            loop {
                count += 1;
                if Process32NextW(snapshot, &mut entry) == 0 {
                    break;
                }
            }
        }
        CloseHandle(snapshot);
        count
    }
}

#[cfg(not(windows))]
pub fn process_count() -> u32 {
    0
}
