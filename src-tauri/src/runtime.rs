use crate::models::{RuntimeCandidate, RuntimeInfo, RuntimeMeta};
use regex::Regex;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;
use wait_timeout::ChildExt;

#[derive(Debug, Clone, Copy)]
pub struct RuntimeSpec {
    pub id: &'static str,
    pub name: &'static str,
    pub extensions: &'static [&'static str],
    pub executables: &'static [&'static str],
    pub version_args: &'static [&'static str],
    pub version_regex: Option<&'static str>,
    pub always_available: bool,
}

pub const SPECS: &[RuntimeSpec] = &[
    RuntimeSpec {
        id: "python",
        name: "Python",
        extensions: &["py", "pyw"],
        executables: &["python.exe", "python3.exe", "python", "py.exe", "py"],
        version_args: &["-V"],
        version_regex: Some(r"Python (\d+\.\d+\.\d+)"),
        always_available: false,
    },
    RuntimeSpec {
        id: "javascript",
        name: "JavaScript (Node.js)",
        extensions: &["js", "mjs", "cjs"],
        executables: &["node.exe", "node"],
        version_args: &["-v"],
        version_regex: Some(r"v(\d+\.\d+\.\d+)"),
        always_available: false,
    },
    RuntimeSpec {
        id: "shell",
        name: "Shell (Git Bash)",
        extensions: &["sh", "bash"],
        executables: &["bash.exe", "bash"],
        version_args: &["--version"],
        version_regex: Some(r"version (\d+\.\d+\.\d+)"),
        always_available: false,
    },
    RuntimeSpec {
        id: "perl",
        name: "Perl",
        extensions: &["pl", "pm"],
        executables: &["perl.exe", "perl"],
        version_args: &["-v"],
        version_regex: Some(r"v(\d+\.\d+\.\d+)"),
        always_available: false,
    },
    RuntimeSpec {
        id: "php",
        name: "PHP",
        extensions: &["php"],
        executables: &["php.exe", "php"],
        version_args: &["-v"],
        version_regex: Some(r"PHP (\d+\.\d+\.\d+)"),
        always_available: false,
    },
    RuntimeSpec {
        id: "r",
        name: "R",
        extensions: &["R", "r"],
        executables: &["Rscript.exe", "Rscript"],
        version_args: &["--version"],
        version_regex: Some(r"version (\d+\.\d+\.\d+)"),
        always_available: false,
    },
    RuntimeSpec {
        id: "lua",
        name: "Lua",
        extensions: &["lua"],
        executables: &["lua.exe", "lua54.exe", "lua53.exe", "luajit.exe", "lua"],
        version_args: &["-v"],
        version_regex: Some(r"(\d+\.\d+\.\d+)"),
        always_available: false,
    },
    RuntimeSpec {
        id: "batch",
        name: "Batch",
        extensions: &["bat", "cmd"],
        executables: &["cmd.exe"],
        version_args: &["/d", "/c", "ver"],
        version_regex: Some(r"Version (\d+\.\d+\.\d+)"),
        always_available: true,
    },
    RuntimeSpec {
        id: "powershell",
        name: "PowerShell",
        extensions: &["ps1", "psm1"],
        executables: &["pwsh.exe", "powershell.exe", "pwsh", "powershell"],
        version_args: &[
            "-NoLogo",
            "-NoProfile",
            "-Command",
            "$PSVersionTable.PSVersion.ToString()",
        ],
        version_regex: Some(r"(\d+\.\d+\.\d+)"),
        always_available: false,
    },
    RuntimeSpec {
        id: "vbscript",
        name: "VBScript",
        extensions: &["vbs", "vbe"],
        executables: &["cscript.exe", "cscript"],
        version_args: &[],
        version_regex: None,
        always_available: false,
    },
    RuntimeSpec {
        id: "jscript",
        name: "JScript",
        extensions: &["js", "jse"],
        executables: &["cscript.exe", "cscript"],
        version_args: &[],
        version_regex: None,
        always_available: false,
    },
    RuntimeSpec {
        id: "wsf",
        name: "Windows Script File",
        extensions: &["wsf"],
        executables: &["cscript.exe", "cscript"],
        version_args: &[],
        version_regex: None,
        always_available: false,
    },
    RuntimeSpec {
        id: "autohotkey",
        name: "AutoHotkey",
        extensions: &["ahk"],
        executables: &["AutoHotkey.exe", "AutoHotkey64.exe", "AutoHotkey32.exe"],
        version_args: &[],
        version_regex: None,
        always_available: false,
    },
    RuntimeSpec {
        id: "autoit",
        name: "AutoIt",
        extensions: &["au3"],
        executables: &["AutoIt3.exe", "AutoIt3_x64.exe"],
        version_args: &[],
        version_regex: None,
        always_available: false,
    },
];

pub fn find_spec(id: &str) -> Option<&'static RuntimeSpec> {
    SPECS.iter().find(|s| s.id.eq_ignore_ascii_case(id))
}

pub fn spec_for_extension(extension: &str) -> Option<&'static RuntimeSpec> {
    let ext = extension.trim_start_matches('.').to_ascii_lowercase();
    SPECS
        .iter()
        .find(|s| s.extensions.iter().any(|e| e.eq_ignore_ascii_case(&ext)))
}

pub fn catalog() -> Vec<RuntimeMeta> {
    SPECS
        .iter()
        .map(|s| RuntimeMeta {
            id: s.id.to_string(),
            name: s.name.to_string(),
            extensions: s.extensions.iter().map(|e| e.to_string()).collect(),
        })
        .collect()
}

fn comspec() -> Option<PathBuf> {
    if let Ok(value) = std::env::var("COMSPEC") {
        let path = PathBuf::from(value);
        if path.is_file() {
            return Some(path);
        }
    }
    let fallback = PathBuf::from(r"C:\Windows\System32\cmd.exe");
    fallback.is_file().then_some(fallback)
}

fn expand_env(input: &str) -> String {
    let mut output = input.to_string();
    for (key, value) in std::env::vars() {
        let needle = format!("%{key}%").to_lowercase();
        loop {
            let lower = output.to_lowercase();
            let Some(position) = lower.find(&needle) else {
                break;
            };
            output.replace_range(position..position + needle.len(), &value);
        }
    }
    output
}

fn push_candidate(paths: &mut Vec<PathBuf>, path: PathBuf) {
    if !path.is_file() {
        return;
    }
    let path = std::fs::canonicalize(&path).unwrap_or(path);
    let key = path.to_string_lossy().to_lowercase();
    if !paths
        .iter()
        .any(|existing| existing.to_string_lossy().to_lowercase() == key)
    {
        paths.push(path);
    }
}

fn push_glob(paths: &mut Vec<PathBuf>, pattern: &str) {
    let expanded = expand_env(pattern);
    if let Ok(entries) = glob::glob(&expanded) {
        for path in entries.flatten() {
            push_candidate(paths, path);
        }
    }
}

fn is_windows_store_alias(path: &Path, spec: &RuntimeSpec) -> bool {
    if !matches!(spec.id, "python" | "javascript") {
        return false;
    }
    path.to_string_lossy()
        .to_lowercase()
        .contains("\\windowsapps\\")
}

#[cfg(windows)]
fn registry_candidates(spec: &RuntimeSpec) -> Vec<PathBuf> {
    use winreg::RegKey;
    use winreg::enums::{
        HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, KEY_WOW64_32KEY, KEY_WOW64_64KEY,
    };

    let mut paths = Vec::new();
    let hives = [HKEY_LOCAL_MACHINE, HKEY_CURRENT_USER];

    // App Paths is the standard registry location for executables registered by installers.
    for hive in hives {
        let root = RegKey::predef(hive);
        for exe in spec.executables {
            for prefix in [
                "SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\App Paths\\",
                "SOFTWARE\\WOW6432Node\\Microsoft\\Windows\\CurrentVersion\\App Paths\\",
            ] {
                if let Ok(key) = root.open_subkey_with_flags(format!("{prefix}{exe}"), KEY_READ) {
                    if let Ok(path) = key.get_value::<String, _>("") {
                        push_candidate(&mut paths, PathBuf::from(path));
                    }
                }
            }
        }
    }

    if spec.id == "python" {
        for hive in hives {
            let root = RegKey::predef(hive);
            for flags in [KEY_READ | KEY_WOW64_64KEY, KEY_READ | KEY_WOW64_32KEY] {
                if let Ok(core) = root.open_subkey_with_flags("SOFTWARE\\Python\\PythonCore", flags)
                {
                    for version in core.enum_keys().flatten() {
                        if let Ok(install) =
                            core.open_subkey_with_flags(format!("{version}\\InstallPath"), flags)
                        {
                            if let Ok(directory) = install.get_value::<String, _>("") {
                                push_candidate(
                                    &mut paths,
                                    PathBuf::from(directory).join("python.exe"),
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    if spec.id == "r" {
        for hive in hives {
            let root = RegKey::predef(hive);
            for key_path in ["SOFTWARE\\R-core\\R", "SOFTWARE\\WOW6432Node\\R-core\\R"] {
                if let Ok(key) = root.open_subkey_with_flags(key_path, KEY_READ) {
                    if let Ok(directory) = key.get_value::<String, _>("InstallPath") {
                        push_candidate(
                            &mut paths,
                            PathBuf::from(&directory).join("bin").join("Rscript.exe"),
                        );
                        push_candidate(
                            &mut paths,
                            PathBuf::from(&directory)
                                .join("bin")
                                .join("x64")
                                .join("Rscript.exe"),
                        );
                    }
                }
            }
        }
    }

    if spec.id == "autohotkey" {
        for hive in hives {
            let root = RegKey::predef(hive);
            for key_path in ["SOFTWARE\\AutoHotkey", "SOFTWARE\\WOW6432Node\\AutoHotkey"] {
                if let Ok(key) = root.open_subkey_with_flags(key_path, KEY_READ) {
                    if let Ok(directory) = key.get_value::<String, _>("InstallDir") {
                        for exe in ["AutoHotkey.exe", "AutoHotkey64.exe", "AutoHotkey32.exe"] {
                            push_candidate(&mut paths, PathBuf::from(&directory).join(exe));
                        }
                    }
                }
            }
        }
    }

    if spec.id == "autoit" {
        for hive in hives {
            let root = RegKey::predef(hive);
            for key_path in [
                "SOFTWARE\\AutoIt v3\\AutoIt",
                "SOFTWARE\\WOW6432Node\\AutoIt v3\\AutoIt",
            ] {
                if let Ok(key) = root.open_subkey_with_flags(key_path, KEY_READ) {
                    if let Ok(directory) = key.get_value::<String, _>("InstallDir") {
                        for exe in ["AutoIt3.exe", "AutoIt3_x64.exe"] {
                            push_candidate(&mut paths, PathBuf::from(&directory).join(exe));
                        }
                    }
                }
            }
        }
    }

    paths
}

#[cfg(not(windows))]
fn registry_candidates(_spec: &RuntimeSpec) -> Vec<PathBuf> {
    Vec::new()
}

fn common_patterns(spec: &RuntimeSpec) -> Vec<&'static str> {
    match spec.id {
        "python" => vec![
            "%LOCALAPPDATA%\\Programs\\Python\\Python*\\python.exe",
            "%ProgramFiles%\\Python*\\python.exe",
            "C:\\Python*\\python.exe",
        ],
        "javascript" => vec![
            "%ProgramFiles%\\nodejs\\node.exe",
            "%LOCALAPPDATA%\\Programs\\nodejs\\node.exe",
        ],
        "shell" => vec![
            "%ProgramFiles%\\Git\\bin\\bash.exe",
            "%ProgramFiles(x86)%\\Git\\bin\\bash.exe",
            "%LOCALAPPDATA%\\Programs\\Git\\bin\\bash.exe",
        ],
        "perl" => vec![
            "C:\\Strawberry\\perl\\bin\\perl.exe",
            "C:\\Perl64\\bin\\perl.exe",
        ],
        "php" => vec![
            "C:\\xampp\\php\\php.exe",
            "C:\\laragon\\bin\\php\\php*\\php.exe",
            "C:\\wamp64\\bin\\php\\php*\\php.exe",
            "C:\\wamp\\bin\\php\\php*\\php.exe",
        ],
        "r" => vec![
            "%ProgramFiles%\\R\\R-*\\bin\\Rscript.exe",
            "%ProgramFiles%\\R\\R-*\\bin\\x64\\Rscript.exe",
        ],
        "lua" => vec![
            "%ProgramFiles%\\Lua\\*\\lua.exe",
            "%LOCALAPPDATA%\\Programs\\Lua\\*\\lua.exe",
        ],
        "powershell" => vec![
            "%SystemRoot%\\System32\\WindowsPowerShell\\v1.0\\powershell.exe",
            "%ProgramFiles%\\PowerShell\\7\\pwsh.exe",
            "%ProgramFiles%\\PowerShell\\7-preview\\pwsh.exe",
        ],
        "vbscript" | "jscript" | "wsf" => vec!["%SystemRoot%\\System32\\cscript.exe"],
        "autohotkey" => vec![
            "%ProgramFiles%\\AutoHotkey\\AutoHotkey.exe",
            "%ProgramFiles%\\AutoHotkey\\v2\\AutoHotkey.exe",
            "%ProgramFiles%\\AutoHotkey\\v2\\AutoHotkey64.exe",
            "%ProgramFiles%\\AutoHotkey\\v1.1.33.09\\AutoHotkeyU64.exe",
        ],
        "autoit" => vec![
            "%ProgramFiles%\\AutoIt3\\AutoIt3.exe",
            "%ProgramFiles(x86)%\\AutoIt3\\AutoIt3.exe",
            "%ProgramFiles%\\AutoIt3\\AutoIt3_x64.exe",
        ],
        _ => Vec::new(),
    }
}

fn candidates_for(spec: &RuntimeSpec) -> Vec<PathBuf> {
    let mut paths = Vec::new();

    // PATH entries first; this preserves the user's active environment.
    for name in spec.executables {
        if let Ok(path) = which::which(name) {
            if !is_windows_store_alias(&path, spec) {
                push_candidate(&mut paths, path);
            }
        }
    }

    if spec.id == "python" {
        for path in python_launcher_candidates() {
            push_candidate(&mut paths, path);
        }
    }

    // Registry entries and typical installer directories cover processes launched
    // from a GUI where the user PATH may not include the runtime.
    for path in registry_candidates(spec) {
        push_candidate(&mut paths, path);
    }
    for pattern in common_patterns(spec) {
        push_glob(&mut paths, pattern);
    }

    if spec.always_available || spec.id == "batch" {
        if let Some(path) = comspec() {
            push_candidate(&mut paths, path);
        }
    }

    paths
}

#[cfg(windows)]
fn python_launcher_candidates() -> Vec<PathBuf> {
    let Some(launcher) = which::which("py.exe").or_else(|_| which::which("py")).ok() else {
        return Vec::new();
    };
    let Some((stdout, _)) = run_capture(&launcher, &["-0p"], Duration::from_millis(1500)) else {
        return Vec::new();
    };
    let Ok(regex) = Regex::new(r"(?i)([A-Z]:\\[^\r\n]*?python(?:w)?\.exe)") else {
        return Vec::new();
    };
    let mut paths = Vec::new();
    for captures in regex.captures_iter(&stdout) {
        if let Some(path) = captures.get(1) {
            push_candidate(&mut paths, PathBuf::from(path.as_str().trim()));
        }
    }
    paths
}

#[cfg(not(windows))]
fn python_launcher_candidates() -> Vec<PathBuf> {
    Vec::new()
}

fn python_gui_paths(paths: &[PathBuf]) -> Vec<PathBuf> {
    let mut values = Vec::new();
    for path in paths {
        if path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.eq_ignore_ascii_case("python.exe"))
        {
            if let Some(parent) = path.parent() {
                push_candidate(&mut values, parent.join("pythonw.exe"));
            }
        }
    }
    values
}

#[cfg(windows)]
fn wsl_distros() -> Vec<String> {
    let Ok(output) = Command::new("wsl.exe")
        .args(["-l", "-q"])
        .stdin(Stdio::null())
        .stderr(Stdio::piped())
        .output()
    else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }
    crate::logging::decode_output(&output.stdout)
        .lines()
        .map(|line| line.trim().trim_matches('\0').to_string())
        .filter(|line| !line.is_empty())
        .collect()
}

#[cfg(not(windows))]
fn wsl_distros() -> Vec<String> {
    Vec::new()
}

#[cfg(windows)]
fn wsh_disabled() -> Option<String> {
    use winreg::RegKey;
    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ};
    for hive in [HKEY_LOCAL_MACHINE, HKEY_CURRENT_USER] {
        let root = RegKey::predef(hive);
        for key_path in [
            "SOFTWARE\\Microsoft\\Windows Script Host\\Settings",
            "SOFTWARE\\Policies\\Microsoft\\Windows Script Host\\Settings",
        ] {
            if let Ok(key) = root.open_subkey_with_flags(key_path, KEY_READ) {
                if let Ok(enabled) = key.get_value::<u32, _>("Enabled") {
                    if enabled == 0 {
                        return Some(format!("WSH 已被注册表策略禁用 ({key_path})"));
                    }
                }
            }
        }
    }
    None
}

#[cfg(not(windows))]
fn wsh_disabled() -> Option<String> {
    None
}

#[cfg(windows)]
fn path_details() -> Vec<String> {
    use winreg::RegKey;
    use winreg::enums::KEY_WOW64_64KEY;
    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ};
    let mut details = Vec::new();
    if let Ok(path) = std::env::var("PATH") {
        details.push(format!(
            "当前进程 PATH: {} 项",
            path.split(';')
                .filter(|value| !value.trim().is_empty())
                .count()
        ));
    }
    let root = RegKey::predef(HKEY_CURRENT_USER);
    if let Ok(environment) = root.open_subkey_with_flags("Environment", KEY_READ) {
        if let Ok(value) = environment.get_value::<String, _>("Path") {
            details.push(format!(
                "用户 PATH: {} 项",
                value
                    .split(';')
                    .filter(|item| !item.trim().is_empty())
                    .count()
            ));
        }
    }
    let root = RegKey::predef(HKEY_LOCAL_MACHINE);
    if let Ok(environment) = root.open_subkey_with_flags(
        "SYSTEM\\CurrentControlSet\\Control\\Session Manager\\Environment",
        KEY_READ | KEY_WOW64_64KEY,
    ) {
        if let Ok(value) = environment.get_value::<String, _>("Path") {
            details.push(format!(
                "系统 PATH: {} 项",
                value
                    .split(';')
                    .filter(|item| !item.trim().is_empty())
                    .count()
            ));
        }
    }
    details
}

#[cfg(not(windows))]
fn path_details() -> Vec<String> {
    Vec::new()
}

fn candidate_source(path: &Path, spec: &RuntimeSpec) -> String {
    if spec.id == "python"
        && path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.eq_ignore_ascii_case("py.exe"))
    {
        return "py launcher".into();
    }
    if let Ok(path_value) = std::env::var("PATH") {
        let key = path.to_string_lossy().to_lowercase();
        if path_value
            .split(';')
            .any(|root| key.starts_with(&root.trim().to_lowercase()))
        {
            return "PATH".into();
        }
    }
    if path
        .to_string_lossy()
        .to_lowercase()
        .contains("\\program files")
    {
        return "installer".into();
    }
    "auto".into()
}

fn candidate_kind(path: &Path) -> String {
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if name.contains("pythonw") {
        "gui".into()
    } else if name.starts_with("wsl") {
        "wsl".into()
    } else {
        "console".into()
    }
}

fn signature_for(paths: &[PathBuf]) -> String {
    paths
        .iter()
        .map(|path| {
            let metadata = std::fs::metadata(path).ok();
            let len = metadata.as_ref().map(|value| value.len()).unwrap_or(0);
            let modified = metadata
                .and_then(|value| value.modified().ok())
                .and_then(|value| value.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|value| value.as_secs())
                .unwrap_or(0);
            format!("{}|{len}|{modified}", path.to_string_lossy())
        })
        .collect::<Vec<_>>()
        .join("\u{1f}")
}

pub fn cached_signature_valid(info: &RuntimeInfo) -> bool {
    if !info.installed {
        return true;
    }
    if info.signature.is_empty() {
        return true;
    }
    info.signature
        .split('\u{1f}')
        .filter(|entry| !entry.is_empty())
        .all(|entry| {
            let mut parts = entry.rsplitn(3, '|');
            let modified = parts.next().and_then(|value| value.parse::<u64>().ok());
            let len = parts.next().and_then(|value| value.parse::<u64>().ok());
            let path = parts.next();
            let (Some(path), Some(len), Some(modified)) = (path, len, modified) else {
                return true;
            };
            let Ok(metadata) = std::fs::metadata(path) else {
                return false;
            };
            let current_modified = metadata
                .modified()
                .ok()
                .and_then(|value| value.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|value| value.as_secs())
                .unwrap_or(0);
            metadata.len() == len && current_modified == modified
        })
}

fn venv_candidates(spec: &RuntimeSpec, script_path: Option<&str>) -> Vec<PathBuf> {
    if spec.id != "python" {
        return Vec::new();
    }
    let Some(script_path) = script_path else {
        return Vec::new();
    };
    let Some(mut directory) = Path::new(script_path).parent() else {
        return Vec::new();
    };

    let mut paths = Vec::new();
    loop {
        for name in [".venv", "venv"] {
            #[cfg(windows)]
            let executable = directory.join(name).join("Scripts").join("python.exe");
            #[cfg(not(windows))]
            let executable = directory.join(name).join("bin").join("python");
            push_candidate(&mut paths, executable);
        }
        match directory.parent() {
            Some(parent) => directory = parent,
            None => break,
        }
    }
    paths
}

pub fn resolve_executable(
    spec: &RuntimeSpec,
    override_path: Option<&str>,
    script_path: Option<&str>,
) -> Result<PathBuf, String> {
    if let Some(path) = override_path {
        let path = PathBuf::from(path);
        if path.is_file() {
            return Ok(path);
        }
        return Err(format!("指定的解释器不存在: {}", path.display()));
    }

    if let Some(path) = venv_candidates(spec, script_path).into_iter().next() {
        return Ok(path);
    }

    if spec.id == "batch" {
        if let Some(path) = comspec() {
            return Ok(path);
        }
    }

    let candidates = candidates_for(spec);
    if spec.id == "autohotkey" {
        if let Some(script_path) = script_path {
            let requires_v2 = std::fs::read(script_path)
                .ok()
                .and_then(|bytes| String::from_utf8(bytes.into_iter().take(8192).collect()).ok())
                .is_some_and(|text| {
                    text.lines()
                        .find(|line| line.trim_start().starts_with("#Requires"))
                        .is_some_and(|line| line.to_ascii_lowercase().contains("v2"))
                });
            for path in &candidates {
                let version = probe_version(path, spec);
                let is_v2 = version
                    .as_deref()
                    .is_some_and(|value| value.starts_with('2'));
                if is_v2 == requires_v2 {
                    return Ok(path.clone());
                }
            }
            if requires_v2 {
                return Err("#Requires AutoHotkey v2，但没有找到 v2 解释器".into());
            }
        }
    }

    if let Some(path) = candidates.into_iter().next() {
        return Ok(path);
    }

    Err(format!(
        "未找到 {} 运行时，请先在“运行时环境”中检测或在系统中配置 PATH",
        spec.name
    ))
}

fn run_capture(exe: &Path, args: &[&str], timeout: Duration) -> Option<(String, String)> {
    let mut command = Command::new(exe);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }

    let mut child = command.spawn().ok()?;
    let mut stdout = child.stdout.take()?;
    let mut stderr = child.stderr.take()?;

    let status = match child.wait_timeout(timeout) {
        Ok(Some(status)) => status,
        Ok(None) => {
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
        Err(_) => {
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
    };

    let mut stdout_bytes = Vec::new();
    let mut stderr_bytes = Vec::new();
    let _ = stdout.read_to_end(&mut stdout_bytes);
    let _ = stderr.read_to_end(&mut stderr_bytes);

    let _ = status;
    Some((
        crate::logging::decode_output(&stdout_bytes),
        crate::logging::decode_output(&stderr_bytes),
    ))
}

fn parse_version(spec: &RuntimeSpec, text: &str) -> Option<String> {
    if let Some(pattern) = spec.version_regex {
        if let Ok(regex) = Regex::new(pattern) {
            if let Some(captures) = regex.captures(text) {
                return captures.get(1).map(|m| m.as_str().to_string());
            }
        }
    }
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(|line| line.chars().take(120).collect())
}

#[cfg(windows)]
fn probe_file_version(exe: &Path) -> Option<String> {
    use std::collections::HashMap;
    use std::sync::{LazyLock, Mutex};

    static CACHE: LazyLock<Mutex<HashMap<String, Option<String>>>> =
        LazyLock::new(|| Mutex::new(HashMap::new()));
    let key = exe.to_string_lossy().to_string();
    if let Ok(values) = CACHE.lock() {
        if let Some(value) = values.get(&key) {
            return value.clone();
        }
    }

    let escaped = key.replace('\'', "''");
    let script = format!(
        "$item = Get-Item -LiteralPath '{}' -ErrorAction SilentlyContinue; \
         if ($item) {{ $v = $item.VersionInfo.ProductVersion; \
         if (-not $v) {{ $v = $item.VersionInfo.FileVersion }}; $v }}",
        escaped
    );
    let (stdout, _) = run_capture(
        Path::new("powershell.exe"),
        &[
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            &script,
        ],
        Duration::from_millis(1500),
    )
    .unwrap_or_default();
    let version = stdout
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(|line| line.chars().take(120).collect::<String>());

    if let Ok(mut values) = CACHE.lock() {
        values.insert(key, version.clone());
    }
    version
}

#[cfg(not(windows))]
fn probe_file_version(_exe: &Path) -> Option<String> {
    None
}

fn probe_version(exe: &Path, spec: &RuntimeSpec) -> Option<String> {
    if spec.version_args.is_empty() {
        return probe_file_version(exe);
    }
    let (stdout, stderr) = run_capture(exe, spec.version_args, Duration::from_millis(1500))?;
    let text = format!("{stdout}\n{stderr}");
    parse_version(spec, &text)
}

pub fn detect_one(spec: &RuntimeSpec) -> RuntimeInfo {
    let paths = candidates_for(spec);
    let gui_paths = if spec.id == "python" {
        python_gui_paths(&paths)
    } else {
        Vec::new()
    };
    let mut candidates = Vec::new();
    let mut first_version = None;

    for (index, path) in paths.iter().enumerate() {
        let version = if index < 4 {
            probe_version(path, spec)
        } else {
            None
        };
        if first_version.is_none() && version.is_some() {
            first_version = version.clone();
        }
        candidates.push(RuntimeCandidate {
            path: path.display().to_string(),
            version,
            source: candidate_source(path, spec),
            kind: candidate_kind(path),
        });
    }
    for (index, path) in gui_paths.iter().enumerate() {
        candidates.push(RuntimeCandidate {
            path: path.display().to_string(),
            version: if index < 2 {
                probe_version(path, spec)
            } else {
                None
            },
            source: candidate_source(path, spec),
            kind: "gui".into(),
        });
    }

    let exe_path = paths.first().map(|path| path.display().to_string());
    let version = first_version.or_else(|| {
        candidates
            .iter()
            .find(|candidate| candidate.kind != "gui")
            .and_then(|candidate| candidate.version.clone())
    });

    let mut details = Vec::new();
    if spec.id == "python" {
        if candidates.iter().any(|candidate| candidate.kind == "gui") {
            details.push("python.exe 用于控制台脚本，pythonw.exe 仅作为 GUI 候选展示".into());
        }
        if candidates
            .iter()
            .any(|candidate| candidate.source == "py launcher")
        {
            details.push("已合并 py -0p 列出的所有 Python 安装".into());
        }
    }
    if spec.id == "shell" {
        let distros = wsl_distros();
        if distros.is_empty() {
            details.push("未检测到 WSL 发行版；当前 Shell 运行时为 Git Bash".into());
        } else {
            details.push(format!(
                "检测到 WSL 发行版: {}（本版本仍使用 Git Bash 执行 .sh）",
                distros.join(", ")
            ));
        }
    }
    if matches!(spec.id, "vbscript" | "jscript" | "wsf") {
        details.push(
            "Windows 11 中 Windows Script Host 为可选功能；若已移除或策略禁用，脚本无法启动".into(),
        );
    }
    details.extend(path_details());

    let mut error = None;
    let mut status = if paths.is_empty() {
        "missing"
    } else if paths.len() > 1 {
        "multiple"
    } else {
        "ready"
    };
    if matches!(spec.id, "vbscript" | "jscript" | "wsf") {
        if let Some(reason) = wsh_disabled() {
            status = "broken";
            error = Some(reason);
        }
    }
    if paths.is_empty() && error.is_none() {
        error = Some("未在 PATH、注册表或常见安装目录中找到可执行文件".to_string());
    }

    let all_paths = paths
        .iter()
        .chain(gui_paths.iter())
        .cloned()
        .collect::<Vec<_>>();

    RuntimeInfo {
        id: spec.id.to_string(),
        name: spec.name.to_string(),
        installed: !paths.is_empty(),
        exe_path,
        version,
        status: status.to_string(),
        candidates,
        error,
        details,
        signature: signature_for(&all_paths),
    }
}

pub fn detect_all() -> Vec<RuntimeInfo> {
    std::thread::scope(|scope| {
        let handles = SPECS
            .iter()
            .map(|spec| scope.spawn(move || detect_one(spec)))
            .collect::<Vec<_>>();
        handles
            .into_iter()
            .map(|handle| {
                handle.join().unwrap_or_else(|_| RuntimeInfo {
                    id: "unknown".into(),
                    name: "Unknown".into(),
                    installed: false,
                    exe_path: None,
                    version: None,
                    status: "broken".into(),
                    candidates: Vec::new(),
                    error: Some("运行时检测线程异常退出".into()),
                    details: Vec::new(),
                    signature: String::new(),
                })
            })
            .collect()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_all_supported_runtimes() {
        let detected = detect_all();
        assert_eq!(detected.len(), 14);

        for runtime in detected {
            assert!(!runtime.id.is_empty());
            if runtime.installed {
                assert!(runtime.exe_path.is_some(), "{} 缺少路径", runtime.name);
            }
            eprintln!(
                "{:<12} {:<8} {:?} {:?}",
                runtime.id, runtime.status, runtime.version, runtime.exe_path
            );
        }
    }
}
