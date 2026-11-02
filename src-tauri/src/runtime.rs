use crate::models::{RuntimeCandidate, RuntimeInfo, RuntimeMeta};
use regex::Regex;
use std::path::PathBuf;
use std::process::{Command, Stdio};

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
    RuntimeSpec { id: "python", name: "Python", extensions: &["py", "pyw"], executables: &["python.exe", "python3.exe"], version_args: &["-V"], version_regex: Some(r"Python (\d+\.\d+\.\d+)"), always_available: false },
    RuntimeSpec { id: "javascript", name: "JavaScript (Node.js)", extensions: &["js", "mjs", "cjs"], executables: &["node.exe", "node"], version_args: &["-v"], version_regex: Some(r"v(\d+\.\d+\.\d+)"), always_available: false },
    RuntimeSpec { id: "shell", name: "Shell (Git Bash)", extensions: &["sh", "bash"], executables: &["bash.exe", "bash"], version_args: &["--version"], version_regex: Some(r"version (\d+\.\d+\.\d+)"), always_available: false },
    RuntimeSpec { id: "perl", name: "Perl", extensions: &["pl", "pm"], executables: &["perl.exe", "perl"], version_args: &["-v"], version_regex: Some(r"v(\d+\.\d+\.\d+)"), always_available: false },
    RuntimeSpec { id: "php", name: "PHP", extensions: &["php"], executables: &["php.exe", "php"], version_args: &["-v"], version_regex: Some(r"PHP (\d+\.\d+\.\d+)"), always_available: false },
    RuntimeSpec { id: "r", name: "R", extensions: &["R", "r"], executables: &["Rscript.exe", "Rscript"], version_args: &["--version"], version_regex: Some(r"version (\d+\.\d+\.\d+)"), always_available: false },
    RuntimeSpec { id: "lua", name: "Lua", extensions: &["lua"], executables: &["lua.exe", "lua54.exe", "lua53.exe", "luajit.exe", "lua"], version_args: &["-v"], version_regex: Some(r"(\d+\.\d+\.\d+)"), always_available: false },
    RuntimeSpec { id: "batch", name: "Batch", extensions: &["bat", "cmd"], executables: &["cmd.exe"], version_args: &["/d", "/c", "ver"], version_regex: Some(r"Version (\d+\.\d+\.\d+)"), always_available: true },
    RuntimeSpec { id: "powershell", name: "PowerShell", extensions: &["ps1", "psm1"], executables: &["pwsh.exe", "powershell.exe", "pwsh", "powershell"], version_args: &["-NoLogo", "-NoProfile", "-Command", "$PSVersionTable.PSVersion.ToString()"], version_regex: Some(r"(\d+\.\d+\.\d+)"), always_available: false },
    RuntimeSpec { id: "vbscript", name: "VBScript", extensions: &["vbs", "vbe"], executables: &["cscript.exe", "cscript"], version_args: &[], version_regex: None, always_available: false },
    RuntimeSpec { id: "jscript", name: "JScript", extensions: &["js", "jse"], executables: &["cscript.exe", "cscript"], version_args: &[], version_regex: None, always_available: false },
    RuntimeSpec { id: "wsf", name: "Windows Script File", extensions: &["wsf"], executables: &["cscript.exe", "cscript"], version_args: &[], version_regex: None, always_available: false },
    RuntimeSpec { id: "autohotkey", name: "AutoHotkey", extensions: &["ahk"], executables: &["AutoHotkey.exe", "AutoHotkey64.exe", "AutoHotkey32.exe"], version_args: &[], version_regex: None, always_available: false },
    RuntimeSpec { id: "autoit", name: "AutoIt", extensions: &["au3"], executables: &["AutoIt3.exe", "AutoIt3_x64.exe"], version_args: &[], version_regex: None, always_available: false },
];

pub fn find_spec(id: &str) -> Option<&'static RuntimeSpec> {
    SPECS.iter().find(|s| s.id.eq_ignore_ascii_case(id))
}

pub fn spec_for_extension(extension: &str) -> Option<&'static RuntimeSpec> {
    let ext = extension.trim_start_matches('.').to_ascii_lowercase();
    SPECS.iter().find(|s| s.extensions.iter().any(|e| e.eq_ignore_ascii_case(&ext)))
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
        if path.exists() {
            return Some(path);
        }
    }
    let fallback = PathBuf::from(r"C:\Windows\System32\cmd.exe");
    fallback.exists().then_some(fallback)
}

pub fn resolve_executable(spec: &RuntimeSpec, override_path: Option<&str>) -> Result<PathBuf, String> {
    if let Some(path) = override_path {
        let path = PathBuf::from(path);
        if path.exists() {
            return Ok(path);
        }
        return Err(format!("指定的解释器不存在: {}", path.display()));
    }
    if spec.id == "batch" {
        if let Some(path) = comspec() {
            return Ok(path);
        }
    }
    for name in spec.executables {
        if let Ok(path) = which::which(name) {
            return Ok(path);
        }
    }
    Err(format!("未找到 {} 运行时，请先在“运行时环境”中检测或在系统中配置 PATH", spec.name))
}

fn probe_version(exe: &PathBuf, spec: &RuntimeSpec) -> Option<String> {
    if spec.version_args.is_empty() {
        return None;
    }
    let mut command = Command::new(exe);
    command.args(spec.version_args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let output = command.output().ok()?;
    let text = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    if let Some(pattern) = spec.version_regex {
        if let Ok(regex) = Regex::new(pattern) {
            if let Some(captures) = regex.captures(&text) {
                return captures.get(1).map(|m| m.as_str().to_string());
            }
        }
    }
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(|line| line.chars().take(120).collect())
}

pub fn detect_one(spec: &RuntimeSpec) -> RuntimeInfo {
    let mut paths: Vec<PathBuf> = Vec::new();
    for name in spec.executables {
        if let Ok(path) = which::which(name) {
            let canonical = std::fs::canonicalize(&path).unwrap_or(path);
            if !paths.iter().any(|p| p == &canonical) {
                paths.push(canonical);
            }
        }
    }
    if paths.is_empty() && spec.always_available {
        if let Some(path) = comspec() {
            paths.push(path);
        }
    }

    let version = paths.first().and_then(|path| probe_version(path, spec));
    let candidates = paths
        .iter()
        .map(|path| RuntimeCandidate {
            path: path.display().to_string(),
            version: if Some(path) == paths.first() { version.clone() } else { None },
        })
        .collect::<Vec<_>>();
    let exe_path = paths.first().map(|p| p.display().to_string());

    RuntimeInfo {
        id: spec.id.to_string(),
        name: spec.name.to_string(),
        installed: !paths.is_empty(),
        exe_path,
        version,
        status: if paths.is_empty() { "missing" } else { "ready" }.to_string(),
        candidates,
        error: None,
    }
}

pub fn detect_all() -> Vec<RuntimeInfo> {
    SPECS.iter().map(detect_one).collect()
}
