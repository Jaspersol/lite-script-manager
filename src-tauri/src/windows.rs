use crate::runner;
use crate::settings;
use crate::state::AppState;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{
    AppHandle, Emitter, Manager, PhysicalPosition, WebviewUrl, WebviewWindow, WebviewWindowBuilder,
};

/// Timestamp of a panel dismissal caused by focus loss.
/// Suppresses the tray click that caused the dismissal.
static PANEL_BLUR_AT: Mutex<Option<Instant>> = Mutex::new(None);
const PANEL_REOPEN_GUARD: Duration = Duration::from_millis(300);
const PANEL_WATCH_INTERVAL: Duration = Duration::from_millis(120);
/// About five seconds of never gaining focus.
const PANEL_FOCUS_GIVE_UP_TICKS: u32 = 40;
static PANEL_GENERATION: AtomicU64 = AtomicU64::new(0);

const PANEL_LABEL: &str = "panel";
const MAIN_LABEL: &str = "main";
const PANEL_WIDTH: f64 = 368.0;
const PANEL_HEIGHT: f64 = 486.0;

pub fn setup_tray(app: &tauri::App) -> tauri::Result<()> {
    let show = MenuItem::with_id(app, "show", "Show LSM", true, None::<&str>)?;
    let check_update = MenuItem::with_id(app, "check-update", "Check updates", true, None::<&str>)?;
    let exit = MenuItem::with_id(app, "exit", "Exit LSM", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &check_update, &exit])?;

    let mut builder = TrayIconBuilder::with_id("lsm-tray")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .tooltip("LiteScriptManager")
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                toggle_panel(tray.app_handle());
            }
        })
        .on_menu_event(|app, event| match event.id.as_ref() {
            "show" => {
                open_main(app);
            }
            "check-update" => {
                check_updates_in_background(app);
            }
            "exit" => exit_app(app),
            _ => {}
        });

    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    builder.build(app)?;
    Ok(())
}

/// Notifies that a newer release is available, in the configured UI language.
pub fn notify_new_version(app: &AppHandle, latest_version: &str) {
    if settings::is_english(app) {
        crate::triggers::notify(
            app,
            "New version available",
            &format!(
                "LiteScriptManager v{latest_version} is available. Install it from Settings → About & updates"
            ),
        );
    } else {
        crate::triggers::notify(
            app,
            "发现新版本",
            &format!("LiteScriptManager v{latest_version} 已发布，可在「设置 → 关于与更新」中安装"),
        );
    }
}

/// Runs a release check from the tray menu and reports the result.
fn check_updates_in_background(app: &AppHandle) {
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        let english = settings::is_english(&handle);
        match crate::update::check(&handle).await {
            Ok(info) => {
                let _ = handle.emit("update-check-result", &info);
                if info.has_update {
                    notify_new_version(&handle, &info.latest_version);
                } else if english {
                    crate::triggers::notify(
                        &handle,
                        "Check for updates",
                        &format!("You're on the latest version, v{}", info.current_version),
                    );
                } else {
                    crate::triggers::notify(
                        &handle,
                        "检查更新",
                        &format!("当前已是最新版本 v{}", info.current_version),
                    );
                }
            }
            Err(error) => {
                let title = if english {
                    "Update check failed"
                } else {
                    "检查更新失败"
                };
                crate::triggers::notify(&handle, title, &error);
            }
        }
    });
}

fn attach_close_behavior(window: &WebviewWindow) {
    let window_for_event = window.clone();
    window.on_window_event(move |event| {
        if let tauri::WindowEvent::CloseRequested { api, .. } = event {
            api.prevent_close();
            if window_for_event.label() == MAIN_LABEL {
                let exit_on_close = window_for_event
                    .app_handle()
                    .try_state::<AppState>()
                    .and_then(|state| {
                        state
                            .db
                            .lock()
                            .ok()
                            .map(|conn| settings::get_bool(&conn, "close_main_exits", false))
                    })
                    .unwrap_or(false);
                if exit_on_close {
                    exit_app(window_for_event.app_handle());
                    return;
                }
            }
            let _ = window_for_event.destroy();
        }
    });
}

pub fn ensure_panel(app: &AppHandle) -> tauri::Result<WebviewWindow> {
    if let Some(window) = app.get_webview_window(PANEL_LABEL) {
        let _ = window.show();
        let _ = window.set_focus();
        return Ok(window);
    }

    let window = WebviewWindowBuilder::new(app, PANEL_LABEL, WebviewUrl::App("panel.html".into()))
        .title("LiteScriptManager")
        .inner_size(PANEL_WIDTH, PANEL_HEIGHT)
        .decorations(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .resizable(false)
        .transparent(true)
        .visible(false)
        .build()?;

    attach_close_behavior(&window);
    let generation = PANEL_GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
    start_dismiss_watchdog(app, &window, generation);
    let position = panel_position(app, &window);
    let _ = window.set_position(position);
    let _ = window.show();
    let _ = window.set_focus();
    Ok(window)
}

/// Dismisses the panel after it has held and then lost focus. The
/// `Focused(false)` event alone is not enough: when Windows refuses the
/// foreground request no later click ever produces a focus event. Polling
/// retries the blocked focus request and gives up after
/// `PANEL_FOCUS_GIVE_UP_TICKS`. A watchdog exits once a newer panel
/// generation exists, so a quick reopen does not leave a stale poller behind.
fn start_dismiss_watchdog(app: &AppHandle, window: &WebviewWindow, generation: u64) {
    let app = app.clone();
    let window = window.clone();
    tauri::async_runtime::spawn(async move {
        let mut had_focus = false;
        let mut focus_attempts = 0u32;
        let mut unfocused_ticks = 0u32;
        loop {
            tokio::time::sleep(PANEL_WATCH_INTERVAL).await;
            if PANEL_GENERATION.load(Ordering::SeqCst) != generation
                || app.get_webview_window(PANEL_LABEL).is_none()
            {
                return;
            }
            if !window.is_visible().unwrap_or(false) {
                had_focus = false;
                focus_attempts = 0;
                unfocused_ticks = 0;
                continue;
            }
            if window.is_focused().unwrap_or(false) {
                had_focus = true;
                unfocused_ticks = 0;
                continue;
            }
            if had_focus {
                if let Ok(mut at) = PANEL_BLUR_AT.lock() {
                    *at = Some(Instant::now());
                }
                let _ = window.destroy();
                return;
            }
            unfocused_ticks += 1;
            if unfocused_ticks >= PANEL_FOCUS_GIVE_UP_TICKS {
                let _ = window.destroy();
                return;
            }
            if focus_attempts < 6 {
                focus_attempts += 1;
                let _ = window.set_focus();
            }
        }
    });
}

pub fn ensure_main(app: &AppHandle) -> tauri::Result<WebviewWindow> {
    if let Some(window) = app.get_webview_window(MAIN_LABEL) {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
        close_panel(app);
        return Ok(window);
    }

    let window = WebviewWindowBuilder::new(app, MAIN_LABEL, WebviewUrl::App("index.html".into()))
        .title("LiteScriptManager")
        .inner_size(1120.0, 760.0)
        .min_inner_size(900.0, 620.0)
        .center()
        .visible(false)
        .disable_drag_drop_handler()
        .build()?;

    apply_light_caption(&window);
    attach_close_behavior(&window);
    let _ = window.show();
    let _ = window.set_focus();
    close_panel(app);
    Ok(window)
}

#[cfg(windows)]
fn apply_light_caption(window: &WebviewWindow) {
    use windows_sys::Win32::Graphics::Dwm::{
        DWMWA_BORDER_COLOR, DWMWA_CAPTION_COLOR, DWMWA_TEXT_COLOR, DwmSetWindowAttribute,
    };

    let Ok(hwnd) = window.hwnd() else {
        return;
    };
    let hwnd = hwnd.0;
    let caption: u32 = 0x00F8F5F4;
    let text: u32 = 0x003B3430;
    let border: u32 = 0x00ECE7E4;
    unsafe {
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_CAPTION_COLOR as u32,
            &caption as *const u32 as *const core::ffi::c_void,
            4,
        );
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_TEXT_COLOR as u32,
            &text as *const u32 as *const core::ffi::c_void,
            4,
        );
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_BORDER_COLOR as u32,
            &border as *const u32 as *const core::ffi::c_void,
            4,
        );
    }
}

#[cfg(not(windows))]
fn apply_light_caption(_window: &WebviewWindow) {}

/// Window creation blocks on WebView2 and deadlocks on Windows when called
/// from the event-loop thread (tray/shortcut/single-instance handlers), so
/// every runtime entry point goes through a worker thread.
fn run_off_main(app: &AppHandle, task: impl FnOnce(&AppHandle) + Send + 'static) {
    let app = app.clone();
    std::thread::spawn(move || task(&app));
}

pub fn open_main(app: &AppHandle) {
    run_off_main(app, |app| {
        let _ = ensure_main(app);
    });
}

pub fn open_panel(app: &AppHandle) {
    run_off_main(app, |app| {
        let _ = ensure_panel(app);
    });
}

pub fn toggle_panel(app: &AppHandle) {
    let recently_dismissed = PANEL_BLUR_AT
        .lock()
        .ok()
        .and_then(|at| *at)
        .is_some_and(|at| at.elapsed() < PANEL_REOPEN_GUARD);
    if recently_dismissed {
        if let Ok(mut at) = PANEL_BLUR_AT.lock() {
            *at = None;
        }
        return;
    }
    run_off_main(app, |app| {
        if let Some(panel) = app.get_webview_window(PANEL_LABEL) {
            let _ = panel.destroy();
        } else {
            let _ = ensure_panel(app);
        }
    });
}

pub fn close_panel(app: &AppHandle) {
    if let Some(panel) = app.get_webview_window(PANEL_LABEL) {
        let _ = panel.destroy();
    }
}

pub fn exit_app(app: &AppHandle) {
    if let Some(state) = app.try_state::<AppState>() {
        state
            .exiting
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }
    runner::stop_all(app, true);
    app.exit(0);
}

fn panel_position(app: &AppHandle, window: &WebviewWindow) -> PhysicalPosition<i32> {
    let Some(monitor) = app.primary_monitor().ok().flatten() else {
        return PhysicalPosition::new(100, 100);
    };
    let scale = monitor.scale_factor();
    let work = monitor.work_area();
    // Use the outer size so DWM frame margins are included.
    let (width, height) = window
        .outer_size()
        .map(|size| (size.width as i32, size.height as i32))
        .unwrap_or(((PANEL_WIDTH * scale) as i32, (PANEL_HEIGHT * scale) as i32));
    let margin = (10.0 * scale) as i32;
    let x = work.position.x + work.size.width as i32 - width - margin;
    let y = work.position.y + work.size.height as i32 - height - margin;
    PhysicalPosition::new(x.max(work.position.x), y.max(work.position.y))
}
