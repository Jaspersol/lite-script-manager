use crate::runner;
use crate::settings;
use crate::state::AppState;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{
    AppHandle, Manager, PhysicalPosition, WebviewUrl, WebviewWindow, WebviewWindowBuilder,
};

/// Timestamp of a panel dismissal caused by focus loss.
/// Suppresses the tray click that caused the dismissal.
static PANEL_BLUR_AT: Mutex<Option<Instant>> = Mutex::new(None);
const PANEL_REOPEN_GUARD: Duration = Duration::from_millis(300);
const BLUR_DEBOUNCE: Duration = Duration::from_millis(150);

const PANEL_LABEL: &str = "panel";
const MAIN_LABEL: &str = "main";
const PANEL_WIDTH: f64 = 368.0;
const PANEL_HEIGHT: f64 = 486.0;

pub fn setup_tray(app: &tauri::App) -> tauri::Result<()> {
    let show = MenuItem::with_id(app, "show", "Show LSM", true, None::<&str>)?;
    let exit = MenuItem::with_id(app, "exit", "Exit LSM", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &exit])?;

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
                let _ = ensure_main(app);
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

fn attach_dismiss_on_blur(window: &WebviewWindow) {
    let window_for_event = window.clone();
    window.on_window_event(move |event| {
        if let tauri::WindowEvent::Focused(false) = event {
            let window = window_for_event.clone();
            // Ignore transient focus loss while the panel is being shown.
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(BLUR_DEBOUNCE).await;
                if window.is_focused().unwrap_or(false) || !window.is_visible().unwrap_or(false) {
                    return;
                }
                if let Ok(mut at) = PANEL_BLUR_AT.lock() {
                    *at = Some(Instant::now());
                }
                let _ = window.destroy();
            });
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
    attach_dismiss_on_blur(&window);
    let position = panel_position(app, &window);
    let _ = window.set_position(position);
    let _ = window.show();
    let _ = window.set_focus();
    Ok(window)
}

pub fn ensure_main(app: &AppHandle) -> tauri::Result<WebviewWindow> {
    if let Some(window) = app.get_webview_window(MAIN_LABEL) {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
        hide_panel(app);
        return Ok(window);
    }

    let window = WebviewWindowBuilder::new(app, MAIN_LABEL, WebviewUrl::App("index.html".into()))
        .title("LiteScriptManager")
        .inner_size(1120.0, 760.0)
        .min_inner_size(900.0, 620.0)
        .center()
        .visible(false)
        .build()?;

    apply_light_caption(&window);
    attach_close_behavior(&window);
    let _ = window.show();
    let _ = window.set_focus();
    hide_panel(app);
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
    if let Some(panel) = app.get_webview_window(PANEL_LABEL) {
        if panel.is_visible().unwrap_or(false) {
            let _ = panel.destroy();
        } else {
            let _ = panel.show();
            let _ = panel.set_focus();
        }
    } else {
        let _ = ensure_panel(app);
    }
}

pub fn close_panel(app: &AppHandle) {
    if let Some(panel) = app.get_webview_window(PANEL_LABEL) {
        let _ = panel.destroy();
    }
}

fn hide_panel(app: &AppHandle) {
    if let Some(panel) = app.get_webview_window(PANEL_LABEL) {
        let _ = panel.hide();
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
