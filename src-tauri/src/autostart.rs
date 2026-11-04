#[cfg(windows)]
pub fn apply(enabled: bool) -> Result<(), String> {
    use winreg::RegKey;
    use winreg::enums::HKEY_CURRENT_USER;
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let (key, _) = hkcu
        .create_subkey("Software\\Microsoft\\Windows\\CurrentVersion\\Run")
        .map_err(|e| format!("打开启动项注册表失败: {e}"))?;
    if enabled {
        // A debug build lives in target\debug; registering it would point
        // Windows startup at a development artifact.
        if cfg!(debug_assertions) {
            return Ok(());
        }
        let exe = std::env::current_exe().map_err(|e| format!("获取程序路径失败: {e}"))?;
        let value = format!("\"{}\"", exe.display());
        key.set_value("LiteScriptManager", &value)
            .map_err(|e| format!("写入启动项失败: {e}"))?;
    } else {
        let _ = key.delete_value("LiteScriptManager");
    }
    Ok(())
}

#[cfg(not(windows))]
pub fn apply(_enabled: bool) -> Result<(), String> {
    Ok(())
}
