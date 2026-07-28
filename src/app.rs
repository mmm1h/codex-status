use crate::app_server::AppServerClient;
#[cfg(feature = "diagnostics")]
use crate::icon::render_bgra_with_overlay;
use crate::icon::{OwnedIcon, ServiceOverlay, create_icon_with_overlay, tone_for_percent};
use crate::insights::{AlertTracker, QuotaKind, analyze_window, current_cycle, evaluate_alerts};
use crate::model::{DisplayState, QuotaSnapshot, QuotaWindow, RefreshState};
use crate::settings::{AppStore, Settings};
use crate::status_page::{
    STATUS_PAGE_HOME, ServiceStatus, ServiceStatusSnapshot, fetch_service_status,
};
use crate::windows_helpers::{
    HotKeyRegistration, PowerBroadcastEvent, SessionChangeEvent, SessionNotificationRegistration,
    power_broadcast_event, session_change_event, write_unicode_text,
};
use crate::{startup, ui, updater};
use chrono::{DateTime, Local, Utc};
#[cfg(feature = "diagnostics")]
use sha2::{Digest, Sha256};
use std::cell::Cell;
use std::mem::size_of;
use std::path::{Path, PathBuf};
use std::ptr;
use std::thread;
use std::time::{Duration, Instant};
use windows::Win32::Foundation::{
    CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE, HINSTANCE, HWND, LPARAM, LRESULT,
    POINT, RECT, SetLastError, WIN32_ERROR, WPARAM,
};
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, HBRUSH, InvalidateRect, MONITOR_DEFAULTTONEAREST, MONITORINFO,
    MonitorFromPoint,
};
use windows::Win32::Media::{TIMERR_NOERROR, timeBeginPeriod, timeEndPeriod};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::ProcessStatus::EmptyWorkingSet;
use windows::Win32::System::Threading::{CreateMutexW, GetCurrentProcess};
use windows::Win32::UI::Controls::WM_MOUSELEAVE;
use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiForMonitor, GetDpiForSystem, GetDpiForWindow,
    GetSystemMetricsForDpi, MDT_EFFECTIVE_DPI, SetProcessDpiAwarenessContext,
};
use windows::Win32::UI::Input::Ime::ImmDisableIME;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, ReleaseCapture, SetCapture, TME_LEAVE, TRACKMOUSEEVENT,
    TrackMouseEvent, VK_ESCAPE,
};
#[cfg(not(codex_status_channel = "portable"))]
use windows::Win32::UI::Shell::NIF_GUID;
use windows::Win32::UI::Shell::{
    NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_SHOWTIP, NIF_TIP, NIIF_INFO, NIIF_RESPECT_QUIET_TIME,
    NIM_ADD, NIM_DELETE, NIM_MODIFY, NIM_SETVERSION, NIN_BALLOONSHOW, NIN_SELECT,
    NOTIFYICON_VERSION_4, NOTIFYICONDATAW, NOTIFYICONIDENTIFIER, Shell_NotifyIconGetRect,
    Shell_NotifyIconW, ShellExecuteW,
};
#[cfg(feature = "diagnostics")]
use windows::Win32::UI::Shell::{NIN_BALLOONHIDE, NIN_BALLOONTIMEOUT, NIN_BALLOONUSERCLICK};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CS_HREDRAW, CS_VREDRAW, CheckMenuRadioItem, CreatePopupMenu, CreateWindowExW,
    DefWindowProcW, DestroyMenu, DestroyWindow, DispatchMessageW, FindWindowW, GetCursorPos,
    GetMessageW, HMENU, IDC_ARROW, IsWindowVisible, KillTimer, LoadCursorW, MB_ICONWARNING, MB_OK,
    MESSAGEBOX_STYLE, MF_BYCOMMAND, MF_CHECKED, MF_DISABLED, MF_GRAYED, MF_POPUP, MF_SEPARATOR,
    MF_STRING, MSG, MessageBoxW, PostMessageW, PostQuitMessage, RegisterClassExW,
    RegisterWindowMessageW, SM_CXSMICON, SW_HIDE, SW_SHOWNORMAL, SWP_NOACTIVATE, SWP_NOZORDER,
    SWP_SHOWWINDOW, SetForegroundWindow, SetTimer, SetWindowPos, ShowWindow, TPM_BOTTOMALIGN,
    TPM_LEFTALIGN, TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenu, TranslateMessage, WA_INACTIVE,
    WINDOW_EX_STYLE, WM_ACTIVATE, WM_APP, WM_CAPTURECHANGED, WM_CLOSE, WM_CONTEXTMENU, WM_DESTROY,
    WM_DISPLAYCHANGE, WM_DPICHANGED, WM_ENDSESSION, WM_ERASEBKGND, WM_HOTKEY, WM_KEYDOWN,
    WM_LBUTTONDBLCLK, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_NULL, WM_PAINT,
    WM_POWERBROADCAST, WM_QUERYENDSESSION, WM_RBUTTONUP, WM_SETTINGCHANGE, WM_TIMER,
    WM_WTSSESSION_CHANGE, WNDCLASSEXW, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_OVERLAPPED, WS_POPUP,
};
#[cfg(feature = "diagnostics")]
use windows::Win32::UI::WindowsAndMessaging::{
    GetMenuItemCount, GetMenuItemID, GetMenuItemInfoW, GetMenuState, GetSubMenu, MENUITEMINFOW,
    MFT_RADIOCHECK, MIIM_FTYPE,
};
#[cfg(not(codex_status_channel = "portable"))]
use windows::core::GUID;
use windows::core::{PCWSTR, w};

#[cfg(codex_status_channel = "stable")]
const MAIN_CLASS: PCWSTR = w!("CodexStatus.MainWindow.v1");
#[cfg(codex_status_channel = "stable")]
const FLYOUT_CLASS: PCWSTR = w!("CodexStatus.FlyoutWindow.v1");
#[cfg(codex_status_channel = "stable")]
const TRAY_GUID: GUID = GUID::from_u128(0x7a89d848_0611_4cb4_98c9_88ca9b59ff84);

#[cfg(codex_status_channel = "beta")]
const MAIN_CLASS: PCWSTR = w!("CodexStatus.Beta.MainWindow.v1");
#[cfg(codex_status_channel = "beta")]
const FLYOUT_CLASS: PCWSTR = w!("CodexStatus.Beta.FlyoutWindow.v1");
#[cfg(codex_status_channel = "beta")]
const TRAY_GUID: GUID = GUID::from_u128(0xcf8c5592_542f_47d2_a7b2_fa3ee023d0b3);

#[cfg(codex_status_channel = "development")]
const MAIN_CLASS: PCWSTR = w!("CodexStatus.Development.MainWindow.v1");
#[cfg(codex_status_channel = "development")]
const FLYOUT_CLASS: PCWSTR = w!("CodexStatus.Development.FlyoutWindow.v1");
#[cfg(codex_status_channel = "development")]
const TRAY_GUID: GUID = GUID::from_u128(0xc4f400e1_9a66_410c_8cd4_babd3aab77b1);

#[cfg(codex_status_channel = "portable")]
const MAIN_CLASS: PCWSTR = w!("CodexStatus.Portable.MainWindow.v1");
#[cfg(codex_status_channel = "portable")]
const FLYOUT_CLASS: PCWSTR = w!("CodexStatus.Portable.FlyoutWindow.v1");

const TRAY_ID: u32 = 1;

const WM_TRAY: u32 = WM_APP + 1;
const WM_REFRESH_COMPLETE: u32 = WM_APP + 2;
const WM_SHOW_EXISTING: u32 = WM_APP + 3;
const WM_TOGGLE_FLYOUT: u32 = WM_APP + 4;
const WM_UPDATE_COMPLETE: u32 = WM_APP + 5;
const WM_STATUS_COMPLETE: u32 = WM_APP + 6;
#[cfg(feature = "diagnostics")]
const WM_DIAGNOSTIC_COMMAND: u32 = WM_APP + 7;
#[cfg(feature = "diagnostics")]
const WM_DIAGNOSTIC_DUMP_MENU: u32 = WM_APP + 8;

const TIMER_REFRESH: usize = 1;
const TIMER_STARTUP: usize = 2;
const TIMER_CARD: usize = 3;
const TIMER_FLYOUT_ACTIVATE: usize = 4;
const TIMER_UPDATE: usize = 5;
const TIMER_WORKING_SET_TRIM: usize = 6;
const TIMER_STATUS: usize = 7;
const TIMER_RENDERER_RELEASE: usize = 8;
const TIMER_REFRESH_ANIMATION: usize = 9;
const TIMER_TEST_NOTIFICATION_FEEDBACK: usize = 10;
const TIMER_REFRESH_WATCHDOG: usize = 11;
const REFRESH_ANIMATION_TARGET_FRAME_MS: u32 = 16;
const REFRESH_ANIMATION_TIMER_MS: u32 = 8;
const REFRESH_ANIMATION_STEP_DEGREES: u16 =
    ((360 * REFRESH_ANIMATION_TARGET_FRAME_MS + 500) / 1_000) as u16;
const REFRESH_WATCHDOG_MS: u32 = 24_000;
const TEST_NOTIFICATION_FEEDBACK_MS: u32 = 2_500;

const UPDATE_INITIAL_DELAY_MS: u32 = 90_000;
const UPDATE_INTERVAL_SECONDS: i64 = 24 * 60 * 60;
const UPDATE_RETRY_MS: u32 = 6 * 60 * 60 * 1_000;
const UPDATE_WORKING_SET_TRIM_MS: u32 = 5_000;
const RENDERER_IDLE_RELEASE_MS: u32 = 3 * 60 * 1_000;
const STATUS_INITIAL_DELAY_MS: u32 = 15_000;
const STATUS_INTERVAL_MS: u32 = 15 * 60 * 1_000;
const STATUS_RETRY_MS: u32 = 30 * 60 * 1_000;
const GLOBAL_HOTKEY_ID: i32 = 1;

const TRAY_ACTIVATION_DEBOUNCE: Duration = Duration::from_millis(300);
const FLYOUT_ACTIVATION_GUARD: Duration = Duration::from_millis(220);
const TRAY_CLOSE_COALESCE: Duration = Duration::from_millis(250);

const CMD_REFRESH: u32 = 100;
const CMD_USAGE: u32 = 101;
const CMD_COPY_STATUS: u32 = 102;
const CMD_COPY_DIAGNOSTICS: u32 = 103;
const CMD_STATUS_PAGE: u32 = 104;
const CMD_STATUS_CHECKS: u32 = 105;
const CMD_TEST_NOTIFICATION: u32 = 106;
const CMD_GLOBAL_HOTKEY: u32 = 107;
const CMD_PIN_FLYOUT: u32 = 108;
const CMD_INTERVAL_1: u32 = 111;
const CMD_INTERVAL_5: u32 = 115;
const CMD_INTERVAL_15: u32 = 125;
const CMD_ALERT_OFF: u32 = 130;
const CMD_ALERT_10: u32 = 131;
const CMD_ALERT_20: u32 = 132;
const CMD_ALERT_30: u32 = 133;
const CMD_SESSION_ALERT_OFF: u32 = 134;
const CMD_SESSION_ALERT_10: u32 = 135;
const CMD_SESSION_ALERT_20: u32 = 136;
const CMD_SESSION_ALERT_30: u32 = 137;
const CMD_PACE_ALERTS: u32 = 138;
const CMD_RECOVERY_ALERTS: u32 = 139;
const CMD_STARTUP: u32 = 140;
const CMD_RELEASES: u32 = 150;
const CMD_CHECK_UPDATES: u32 = 151;
const CMD_THEME_SYSTEM: u32 = 160;
const CMD_THEME_LIGHT: u32 = 161;
const CMD_THEME_DARK: u32 = 162;
const CMD_TRAY_WEEKLY: u32 = 170;
const CMD_TRAY_SESSION: u32 = 171;
const CMD_TRAY_LOWEST: u32 = 172;
const CMD_EXIT: u32 = 199;

const USAGE_URL: &str = "https://chatgpt.com/codex/settings/usage";
const RELEASES_URL: &str = "https://github.com/mmm1h/codex-status/releases";

thread_local! {
    static STATE: Cell<*mut AppState> = const { Cell::new(ptr::null_mut()) };
}

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("CodexStatus received unsupported command-line arguments")]
    InvalidArguments,
    #[error("Windows could not initialize CodexStatus: {0}")]
    Windows(#[from] windows::core::Error),
}

struct InstanceHandle(HANDLE);

impl Drop for InstanceHandle {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

struct RefreshOutcome {
    id: u64,
    result: Result<QuotaSnapshot, String>,
}

#[derive(Debug, Default)]
struct RefreshSequence {
    next_id: u64,
    active_id: Option<u64>,
    pending_force: bool,
}

impl RefreshSequence {
    fn begin(&mut self, force: bool, paused: bool) -> Option<u64> {
        if paused || self.active_id.is_some() {
            self.pending_force |= force;
            return None;
        }
        self.next_id = self.next_id.wrapping_add(1).max(1);
        self.active_id = Some(self.next_id);
        Some(self.next_id)
    }

    fn finish(&mut self, id: u64) -> Option<bool> {
        if self.active_id != Some(id) {
            return None;
        }
        self.active_id = None;
        Some(std::mem::take(&mut self.pending_force))
    }

    const fn is_active(&self) -> bool {
        self.active_id.is_some()
    }

    const fn active_id(&self) -> Option<u64> {
        self.active_id
    }

    fn clear_pending(&mut self) {
        self.pending_force = false;
    }
}

struct UpdateOutcome {
    kind: UpdateCheckKind,
    result: Result<Option<updater::StagedUpdate>, updater::UpdateError>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UpdateCheckKind {
    Automatic,
    Manual,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NotificationKind {
    Alert,
    ActionRequired,
    Test,
}

impl NotificationKind {
    const fn respects_quiet_time(self) -> bool {
        matches!(self, Self::Alert)
    }
}

impl UpdateCheckKind {
    const fn bypasses_daily_throttle(self) -> bool {
        matches!(self, Self::Manual)
    }

    const fn records_last_check(self) -> bool {
        matches!(self, Self::Automatic)
    }
}

struct StatusOutcome {
    result: Result<ServiceStatusSnapshot, String>,
}

enum LaunchMode {
    Normal,
    Background,
    ApplyUpdate { parent_pid: u32, target: PathBuf },
}

struct AppState {
    hwnd: HWND,
    flyout: HWND,
    taskbar_created: u32,
    store: AppStore,
    settings: Settings,
    locale: ui::Locale,
    theme: ui::Theme,
    display: DisplayState,
    client: AppServerClient,
    tray_icon: Option<OwnedIcon>,
    tray_added: bool,
    refresh_sequence: RefreshSequence,
    refresh_hovered: bool,
    refresh_pointer_down: bool,
    refresh_angle_degrees: u16,
    refresh_timer_resolution_active: bool,
    #[cfg(feature = "diagnostics")]
    refresh_animation_started: Option<Instant>,
    #[cfg(feature = "diagnostics")]
    refresh_animation_frames: u32,
    update_checking: bool,
    pending_update: Option<updater::StagedUpdate>,
    pending_update_manual: bool,
    test_notification_pending: bool,
    status_checking: bool,
    service_status: ServiceStatusSnapshot,
    refresh_paused: bool,
    alert_tracker: AlertTracker,
    hotkey: Option<HotKeyRegistration>,
    session_notifications: Option<SessionNotificationRegistration>,
    failures: u8,
    last_tray_activation: Option<Instant>,
    flyout_ignore_inactive_until: Option<Instant>,
    flyout_hidden_for_tray_activation: Option<Instant>,
}

pub fn run() -> Result<(), AppError> {
    diagnostic("run:enter");
    let launch_mode = parse_arguments()?;
    if let LaunchMode::ApplyUpdate { parent_pid, target } = launch_mode {
        updater::apply_update_silently(parent_pid, &target);
        return Ok(());
    }
    let background = matches!(launch_mode, LaunchMode::Background);
    diagnostic("run:arguments");
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        // The app has no editable controls. Disabling text services before the
        // first window is created prevents third-party IME/TIP modules from
        // being injected merely because the flyout receives focus.
        let _ = ImmDisableIME(0);
    }
    unsafe {
        SetLastError(WIN32_ERROR(0));
    }
    let mutex_name = wide0(mutex_name_text());
    let mutex = unsafe { InstanceHandle(CreateMutexW(None, false, PCWSTR(mutex_name.as_ptr()))?) };
    let mutex_was_existing = unsafe { GetLastError() == ERROR_ALREADY_EXISTS };
    diagnostic("run:mutex");
    if mutex_was_existing {
        if let Ok(existing) = unsafe { FindWindowW(MAIN_CLASS, PCWSTR::null()) } {
            unsafe {
                let _ = PostMessageW(Some(existing), WM_SHOW_EXISTING, WPARAM(0), LPARAM(0));
            }
        }
        return Ok(());
    }
    if let Ok(executable) = std::env::current_exe() {
        let _ = startup::migrate_legacy(&executable);
    }

    let instance = unsafe { HINSTANCE(GetModuleHandleW(None)?.0) };
    register_classes(instance)?;
    diagnostic("run:classes");
    let hwnd = unsafe {
        CreateWindowExW(
            WS_EX_TOOLWINDOW,
            MAIN_CLASS,
            w!("CodexStatus"),
            WS_OVERLAPPED,
            0,
            0,
            0,
            0,
            None,
            None,
            Some(instance),
            None,
        )?
    };
    let flyout = unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE(WS_EX_TOOLWINDOW.0 | WS_EX_TOPMOST.0),
            FLYOUT_CLASS,
            w!("CodexStatus"),
            WS_POPUP,
            0,
            0,
            ui::CARD_WIDTH,
            ui::CARD_HEIGHT,
            Some(hwnd),
            None,
            Some(instance),
            None,
        )?
    };
    diagnostic("run:windows");

    let store = AppStore::discover();
    let mut settings = store.load_settings();
    let locale = ui::Locale::detect(&settings.locale);
    let theme = ui::detect_theme(&settings.theme);
    let now = Utc::now().timestamp();
    let cached = store.load_snapshot().filter(|snapshot| snapshot.is_cache_valid(now));
    let display = DisplayState::loading(cached);
    let alert_tracker = seed_alert_tracker(&settings, &display, now);
    let taskbar_created = unsafe { RegisterWindowMessageW(w!("TaskbarCreated")) };
    let session_notifications = SessionNotificationRegistration::register(hwnd).ok();
    let hotkey = if settings.global_hotkey {
        HotKeyRegistration::register(
            Some(hwnd),
            GLOBAL_HOTKEY_ID,
            MOD_ALT | MOD_CONTROL | MOD_NOREPEAT,
            u32::from(b'Q'),
        )
        .ok()
    } else {
        None
    };
    let hotkey_failed = settings.global_hotkey && hotkey.is_none();
    let hotkey_settings_save_error = if hotkey_failed {
        settings.global_hotkey = false;
        store.save_settings(&settings).err()
    } else {
        None
    };
    let state = Box::new(AppState {
        hwnd,
        flyout,
        taskbar_created,
        store,
        settings,
        locale,
        theme,
        display,
        client: AppServerClient::new(),
        tray_icon: None,
        tray_added: false,
        refresh_sequence: RefreshSequence::default(),
        refresh_hovered: false,
        refresh_pointer_down: false,
        refresh_angle_degrees: 0,
        refresh_timer_resolution_active: false,
        #[cfg(feature = "diagnostics")]
        refresh_animation_started: None,
        #[cfg(feature = "diagnostics")]
        refresh_animation_frames: 0,
        update_checking: false,
        pending_update: None,
        pending_update_manual: false,
        test_notification_pending: false,
        status_checking: false,
        service_status: ServiceStatusSnapshot::unavailable(),
        refresh_paused: false,
        alert_tracker,
        hotkey,
        session_notifications,
        failures: 0,
        last_tray_activation: None,
        flyout_ignore_inactive_until: None,
        flyout_hidden_for_tray_activation: None,
    });
    let raw = Box::into_raw(state);
    STATE.with(|slot| slot.set(raw));
    #[cfg(feature = "diagnostics")]
    diagnostic_event(
        "diagnostic_ready",
        serde_json::json!({
            "hwnd": unsafe { (*raw).hwnd.0 as isize },
            "flyout": unsafe { (*raw).flyout.0 as isize },
        }),
    );
    diagnostic("run:state");

    let initialization = unsafe {
        let state = &mut *raw;
        ui::configure_flyout(state.flyout, state.theme);
        diagnostic("run:dwm");
        state.update_tray(true)
    };
    diagnostic("run:tray-returned");
    if let Err(error) = initialization {
        STATE.with(|slot| slot.set(ptr::null_mut()));
        unsafe {
            drop(Box::from_raw(raw));
        }
        return Err(error.into());
    }

    unsafe {
        let state = &mut *raw;
        state.reset_refresh_timer(state.settings.refresh_minutes.saturating_mul(60_000));
        state.schedule_update_check(UPDATE_INITIAL_DELAY_MS);
        if state.settings.service_status_checks {
            state.reset_status_timer(if background { 45_000 } else { STATUS_INITIAL_DELAY_MS });
        }
        if hotkey_failed {
            state.show_balloon(
                state.locale.text("Shortcut unavailable", "快捷键不可用"),
                state.locale.text(
                    "Ctrl+Alt+Q is already used by another app.",
                    "Ctrl+Alt+Q 已被其他应用占用。",
                ),
            );
        }
        if let Some(error) = hotkey_settings_save_error.as_ref() {
            state.show_balloon(
                state.locale.text("Settings not saved", "设置未保存"),
                &error.to_string(),
            );
        }
        if background {
            let _ = SetTimer(Some(hwnd), TIMER_STARTUP, 30_000, None);
        } else {
            state.start_refresh(false);
        }
    }
    diagnostic("run:message-loop");

    let mut message = MSG::default();
    unsafe {
        while GetMessageW(&mut message, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }

    STATE.with(|slot| slot.set(ptr::null_mut()));
    unsafe {
        drop(Box::from_raw(raw));
    }
    drop(mutex);
    Ok(())
}

fn parse_arguments() -> Result<LaunchMode, AppError> {
    let arguments: Vec<_> = std::env::args_os().skip(1).collect();
    match arguments.as_slice() {
        [] => Ok(LaunchMode::Normal),
        [argument] if argument == "--background" => Ok(LaunchMode::Background),
        [mode, parent_pid, target] if mode == "--apply-update" => {
            let parent_pid =
                parent_pid.to_string_lossy().parse().map_err(|_| AppError::InvalidArguments)?;
            Ok(LaunchMode::ApplyUpdate { parent_pid, target: PathBuf::from(target) })
        }
        _ => Err(AppError::InvalidArguments),
    }
}

fn register_classes(instance: HINSTANCE) -> windows::core::Result<()> {
    unsafe {
        let cursor = LoadCursorW(None, IDC_ARROW)?;
        let main = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            hInstance: instance,
            lpszClassName: MAIN_CLASS,
            lpfnWndProc: Some(main_window_proc),
            hCursor: cursor,
            ..Default::default()
        };
        if RegisterClassExW(&main) == 0 {
            return Err(windows::core::Error::from_thread());
        }
        let flyout = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            style: CS_HREDRAW | CS_VREDRAW,
            hInstance: instance,
            lpszClassName: FLYOUT_CLASS,
            lpfnWndProc: Some(flyout_window_proc),
            hCursor: cursor,
            hbrBackground: HBRUSH::default(),
            ..Default::default()
        };
        if RegisterClassExW(&flyout) == 0 {
            return Err(windows::core::Error::from_thread());
        }
    }
    Ok(())
}

unsafe extern "system" fn main_window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    unsafe {
        let state_ptr = STATE.with(Cell::get);
        if !state_ptr.is_null() {
            let state = &mut *state_ptr;
            if message == state.taskbar_created {
                state.tray_added = false;
                let _ = state.update_tray(true);
                return LRESULT(0);
            }
            match message {
                WM_TRAY => {
                    let event = lparam.0 as u32 & 0xffff;
                    if event == NIN_BALLOONSHOW && state.test_notification_pending {
                        state.test_notification_pending = false;
                        let _ = KillTimer(Some(hwnd), TIMER_TEST_NOTIFICATION_FEEDBACK);
                        #[cfg(feature = "diagnostics")]
                        diagnostic_event(
                            "test_notification_feedback",
                            serde_json::json!({ "displayed": true, "fallback": false }),
                        );
                    }
                    #[cfg(feature = "diagnostics")]
                    diagnostic_event(
                        "tray_callback",
                        serde_json::json!({
                            "event": event,
                            "name": match event {
                                NIN_BALLOONSHOW => "balloon_show",
                                NIN_BALLOONHIDE => "balloon_hide",
                                NIN_BALLOONTIMEOUT => "balloon_timeout",
                                NIN_BALLOONUSERCLICK => "balloon_user_click",
                                _ => "other",
                            },
                        }),
                    );
                    match event {
                        WM_LBUTTONUP | WM_LBUTTONDBLCLK | NIN_SELECT => {
                            state.request_toggle_flyout()
                        }
                        WM_RBUTTONUP | WM_CONTEXTMENU => state.show_menu(),
                        _ => {}
                    }
                    return LRESULT(0);
                }
                WM_TOGGLE_FLYOUT => {
                    state.toggle_flyout();
                    return LRESULT(0);
                }
                WM_REFRESH_COMPLETE => {
                    if lparam.0 != 0 {
                        let outcome = *Box::from_raw(lparam.0 as *mut RefreshOutcome);
                        state.finish_refresh(outcome);
                    }
                    return LRESULT(0);
                }
                WM_UPDATE_COMPLETE => {
                    if lparam.0 != 0 {
                        let outcome = *Box::from_raw(lparam.0 as *mut UpdateOutcome);
                        state.finish_update_check(outcome);
                    }
                    return LRESULT(0);
                }
                WM_STATUS_COMPLETE => {
                    if lparam.0 != 0 {
                        let outcome = *Box::from_raw(lparam.0 as *mut StatusOutcome);
                        state.finish_status_check(outcome);
                    }
                    return LRESULT(0);
                }
                #[cfg(feature = "diagnostics")]
                WM_DIAGNOSTIC_COMMAND => {
                    if let Ok(command) = u32::try_from(wparam.0) {
                        diagnostic_event(
                            "diagnostic_command_received",
                            serde_json::json!({ "command": command }),
                        );
                        state.handle_command(command);
                    }
                    return LRESULT(0);
                }
                #[cfg(feature = "diagnostics")]
                WM_DIAGNOSTIC_DUMP_MENU => {
                    state.dump_menu_diagnostics();
                    return LRESULT(0);
                }
                WM_SHOW_EXISTING => {
                    state.show_flyout();
                    return LRESULT(0);
                }
                WM_TIMER => {
                    match wparam.0 {
                        TIMER_REFRESH => state.start_refresh(false),
                        TIMER_STARTUP => {
                            let _ = KillTimer(Some(hwnd), TIMER_STARTUP);
                            state.start_refresh(false);
                        }
                        TIMER_CARD => {
                            state.expire_cache_if_needed();
                            let _ = InvalidateRect(Some(state.flyout), None, false);
                        }
                        TIMER_FLYOUT_ACTIVATE => {
                            let _ = KillTimer(Some(hwnd), TIMER_FLYOUT_ACTIVATE);
                            state.finish_flyout_activation();
                        }
                        TIMER_UPDATE => {
                            let _ = KillTimer(Some(hwnd), TIMER_UPDATE);
                            if state.pending_update.is_some() {
                                state.try_apply_update();
                            } else {
                                state.start_update_check(UpdateCheckKind::Automatic);
                            }
                        }
                        TIMER_WORKING_SET_TRIM => state.trim_working_set(),
                        TIMER_STATUS => {
                            let _ = KillTimer(Some(hwnd), TIMER_STATUS);
                            state.start_status_check();
                        }
                        TIMER_RENDERER_RELEASE => {
                            let _ = KillTimer(Some(hwnd), TIMER_RENDERER_RELEASE);
                            if !IsWindowVisible(state.flyout).as_bool() {
                                ui::release_card_device_tree();
                                state.schedule_working_set_trim();
                            }
                        }
                        TIMER_REFRESH_ANIMATION => {
                            if state.refresh_sequence.is_active()
                                && IsWindowVisible(state.flyout).as_bool()
                            {
                                #[cfg(feature = "diagnostics")]
                                {
                                    state.refresh_animation_frames =
                                        state.refresh_animation_frames.saturating_add(1);
                                }
                                state.refresh_angle_degrees = state
                                    .refresh_angle_degrees
                                    .wrapping_add(REFRESH_ANIMATION_STEP_DEGREES)
                                    % 360;
                                invalidate_refresh_button(
                                    state.flyout,
                                    GetDpiForWindow(state.flyout).max(96),
                                );
                            } else {
                                state.stop_refresh_animation();
                            }
                        }
                        TIMER_REFRESH_WATCHDOG => {
                            let _ = KillTimer(Some(hwnd), TIMER_REFRESH_WATCHDOG);
                            if let Some(id) = state.refresh_sequence.active_id() {
                                #[cfg(feature = "diagnostics")]
                                diagnostic_event(
                                    "refresh_watchdog",
                                    serde_json::json!({ "id": id, "timeout_ms": REFRESH_WATCHDOG_MS }),
                                );
                                state.finish_refresh(RefreshOutcome {
                                    id,
                                    result: Err("Codex refresh timed out before cleanup completed"
                                        .to_owned()),
                                });
                            }
                        }
                        TIMER_TEST_NOTIFICATION_FEEDBACK => {
                            let _ = KillTimer(Some(hwnd), TIMER_TEST_NOTIFICATION_FEEDBACK);
                            if state.test_notification_pending {
                                state.test_notification_pending = false;
                                state.show_test_notification_fallback();
                            }
                        }
                        _ => {}
                    }
                    return LRESULT(0);
                }
                WM_HOTKEY if wparam.0 as i32 == GLOBAL_HOTKEY_ID => {
                    state.toggle_flyout();
                    return LRESULT(0);
                }
                WM_WTSSESSION_CHANGE => {
                    if let Some(event) = session_change_event(message, wparam.0) {
                        state.handle_session_change(event);
                    }
                    // Remote-session transitions can change SM_REMOTESESSION
                    // without a theme preference change. Re-evaluate the
                    // backdrop policy for every WTS session notification.
                    state.theme = ui::detect_theme(&state.settings.theme);
                    ui::configure_flyout(state.flyout, state.theme);
                    let _ = InvalidateRect(Some(state.flyout), None, false);
                    return LRESULT(0);
                }
                WM_POWERBROADCAST => {
                    if let Some(event) = power_broadcast_event(message, wparam.0) {
                        state.handle_power_change(event);
                    }
                    return LRESULT(1);
                }
                WM_SETTINGCHANGE | WM_DISPLAYCHANGE => {
                    state.theme = ui::detect_theme(&state.settings.theme);
                    ui::configure_flyout(state.flyout, state.theme);
                    let _ = state.update_tray(false);
                    let _ = InvalidateRect(Some(state.flyout), None, true);
                    return LRESULT(0);
                }
                WM_QUERYENDSESSION => return LRESULT(1),
                WM_ENDSESSION if wparam.0 != 0 => {
                    let _ = DestroyWindow(hwnd);
                    return LRESULT(0);
                }
                WM_CLOSE => {
                    let _ = DestroyWindow(hwnd);
                    return LRESULT(0);
                }
                WM_DESTROY => {
                    state.hotkey.take();
                    state.session_notifications.take();
                    PostQuitMessage(0);
                    return LRESULT(0);
                }
                _ => {}
            }
        }
        DefWindowProcW(hwnd, message, wparam, lparam)
    }
}

unsafe extern "system" fn flyout_window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    unsafe {
        let state_ptr = STATE.with(Cell::get);
        match message {
            WM_PAINT if !state_ptr.is_null() => {
                let state = &*state_ptr;
                let refresh_button = if state.refresh_pointer_down && state.refresh_hovered {
                    ui::RefreshButtonState::Pressed
                } else if state.refresh_hovered {
                    ui::RefreshButtonState::Hovered
                } else {
                    ui::RefreshButtonState::Idle
                };
                ui::paint_card(
                    hwnd,
                    &state.display,
                    state.locale,
                    state.theme,
                    refresh_button,
                    state.refresh_sequence.is_active(),
                    f32::from(state.refresh_angle_degrees),
                );
                return LRESULT(0);
            }
            WM_MOUSEMOVE if !state_ptr.is_null() => {
                let state = &mut *state_ptr;
                let (x, y) = message_point(lparam);
                let dpi = GetDpiForWindow(hwnd).max(96);
                let hovered = ui::refresh_hit_test(x, y, dpi);
                if hovered != state.refresh_hovered {
                    state.refresh_hovered = hovered;
                    invalidate_refresh_button(hwnd, dpi);
                }
                let mut tracking = TRACKMOUSEEVENT {
                    cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
                    dwFlags: TME_LEAVE,
                    hwndTrack: hwnd,
                    dwHoverTime: 0,
                };
                let _ = TrackMouseEvent(&mut tracking);
                return LRESULT(0);
            }
            WM_MOUSELEAVE if !state_ptr.is_null() => {
                let state = &mut *state_ptr;
                if state.refresh_hovered {
                    state.refresh_hovered = false;
                    invalidate_refresh_button(hwnd, GetDpiForWindow(hwnd).max(96));
                }
                return LRESULT(0);
            }
            WM_LBUTTONDOWN if !state_ptr.is_null() => {
                let state = &mut *state_ptr;
                let (x, y) = message_point(lparam);
                let dpi = GetDpiForWindow(hwnd).max(96);
                if !state.refresh_sequence.is_active() && ui::refresh_hit_test(x, y, dpi) {
                    state.refresh_hovered = true;
                    state.refresh_pointer_down = true;
                    let _ = SetCapture(hwnd);
                    invalidate_refresh_button(hwnd, dpi);
                }
                return LRESULT(0);
            }
            WM_LBUTTONUP if !state_ptr.is_null() => {
                let state = &mut *state_ptr;
                let (x, y) = message_point(lparam);
                let dpi = GetDpiForWindow(hwnd).max(96);
                let hit = ui::refresh_hit_test(x, y, dpi);
                let activate =
                    state.refresh_pointer_down && hit && !state.refresh_sequence.is_active();
                state.refresh_pointer_down = false;
                state.refresh_hovered = hit;
                let _ = ReleaseCapture();
                invalidate_refresh_button(hwnd, dpi);
                if activate {
                    state.start_refresh(true);
                }
                return LRESULT(0);
            }
            WM_CAPTURECHANGED if !state_ptr.is_null() => {
                let state = &mut *state_ptr;
                if state.refresh_pointer_down {
                    state.refresh_pointer_down = false;
                    invalidate_refresh_button(hwnd, GetDpiForWindow(hwnd).max(96));
                }
                return LRESULT(0);
            }
            WM_ERASEBKGND => return LRESULT(1),
            WM_ACTIVATE if (wparam.0 as u32 & 0xffff) == WA_INACTIVE => {
                if !state_ptr.is_null() {
                    let state = &mut *state_ptr;
                    state.handle_flyout_inactive();
                } else {
                    let _ = ShowWindow(hwnd, SW_HIDE);
                }
                return LRESULT(0);
            }
            WM_KEYDOWN if wparam.0 as u16 == VK_ESCAPE.0 => {
                if !state_ptr.is_null() {
                    let state = &mut *state_ptr;
                    state.hide_flyout();
                } else {
                    let _ = ShowWindow(hwnd, SW_HIDE);
                }
                return LRESULT(0);
            }
            WM_CLOSE => {
                if !state_ptr.is_null() {
                    let state = &mut *state_ptr;
                    state.hide_flyout();
                } else {
                    let _ = ShowWindow(hwnd, SW_HIDE);
                }
                return LRESULT(0);
            }
            WM_DPICHANGED => {
                let suggested = &*(lparam.0 as *const RECT);
                let _ = SetWindowPos(
                    hwnd,
                    None,
                    suggested.left,
                    suggested.top,
                    suggested.right - suggested.left,
                    suggested.bottom - suggested.top,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                );
                // The next paint releases the old back-buffer bitmap, resizes
                // the physical-pixel swapchain, and reapplies the new D2D DPI.
                let _ = InvalidateRect(Some(hwnd), None, false);
                return LRESULT(0);
            }
            _ => {}
        }
        DefWindowProcW(hwnd, message, wparam, lparam)
    }
}

fn message_point(lparam: LPARAM) -> (i32, i32) {
    let packed = lparam.0 as u32;
    let x = (packed & 0xffff) as u16 as i16 as i32;
    let y = ((packed >> 16) & 0xffff) as u16 as i16 as i32;
    (x, y)
}

fn invalidate_refresh_button(hwnd: HWND, dpi: u32) {
    let rect = ui::refresh_rect(dpi);
    unsafe {
        let _ = InvalidateRect(Some(hwnd), Some(&rect), false);
    }
}

impl AppState {
    fn reset_status_timer(&self, delay_ms: u32) {
        unsafe {
            let _ = KillTimer(Some(self.hwnd), TIMER_STATUS);
            if self.settings.service_status_checks && !self.refresh_paused {
                let interval = delay_ms.max(1_000);
                let timer = SetTimer(Some(self.hwnd), TIMER_STATUS, interval, None);
                #[cfg(feature = "diagnostics")]
                diagnostic_event(
                    "timer",
                    serde_json::json!({
                        "kind": "service_status",
                        "active": timer != 0,
                        "interval_ms": interval,
                    }),
                );
                #[cfg(not(feature = "diagnostics"))]
                let _ = timer;
            }
        }
    }

    fn start_status_check(&mut self) {
        if self.status_checking || !self.settings.service_status_checks || self.refresh_paused {
            return;
        }
        self.status_checking = true;
        let hwnd_value = self.hwnd.0 as isize;
        let spawn_result = thread::Builder::new()
            .name("codex-status-service".to_owned())
            .stack_size(384 * 1024)
            .spawn(move || {
                let hwnd = HWND(hwnd_value as *mut std::ffi::c_void);
                let outcome = StatusOutcome {
                    result: fetch_service_status().map_err(|error| error.to_string()),
                };
                let raw = Box::into_raw(Box::new(outcome));
                if unsafe {
                    PostMessageW(Some(hwnd), WM_STATUS_COMPLETE, WPARAM(0), LPARAM(raw as isize))
                }
                .is_err()
                {
                    unsafe {
                        drop(Box::from_raw(raw));
                    }
                }
            });
        if spawn_result.is_err() {
            self.status_checking = false;
            self.reset_status_timer(STATUS_RETRY_MS);
        }
    }

    fn finish_status_check(&mut self, outcome: StatusOutcome) {
        self.status_checking = false;
        if !self.settings.service_status_checks {
            self.service_status = ServiceStatusSnapshot::unavailable();
            let _ = self.update_tray(false);
            unsafe {
                let _ = InvalidateRect(Some(self.flyout), None, false);
            }
            return;
        }
        let delay = match outcome.result {
            Ok(snapshot) => {
                self.service_status = snapshot;
                STATUS_INTERVAL_MS
            }
            Err(_) => {
                self.service_status = ServiceStatusSnapshot::unavailable();
                STATUS_RETRY_MS
            }
        };
        self.reset_status_timer(delay);
        let _ = self.update_tray(false);
        unsafe {
            let _ = InvalidateRect(Some(self.flyout), None, false);
        }
        self.schedule_working_set_trim();
    }

    fn handle_session_change(&mut self, event: SessionChangeEvent) {
        self.set_refresh_paused(matches!(event, SessionChangeEvent::Locked));
    }

    fn handle_power_change(&mut self, event: PowerBroadcastEvent) {
        self.set_refresh_paused(matches!(event, PowerBroadcastEvent::Suspend));
    }

    fn set_refresh_paused(&mut self, paused: bool) {
        if self.refresh_paused == paused {
            return;
        }
        self.refresh_paused = paused;
        unsafe {
            let _ = KillTimer(Some(self.hwnd), TIMER_REFRESH);
            let _ = KillTimer(Some(self.hwnd), TIMER_STATUS);
        }
        if paused {
            self.stop_refresh_animation();
        } else {
            self.refresh_sequence.clear_pending();
            self.reset_refresh_timer(self.settings.refresh_minutes.saturating_mul(60_000));
            self.start_refresh(false);
            self.start_refresh_animation();
            if self.settings.service_status_checks {
                self.reset_status_timer(5_000);
            }
        }
    }

    fn schedule_update_check(&self, fallback_delay_ms: u32) {
        if !updater::updates_supported() {
            return;
        }
        let now = Utc::now().timestamp();
        let delay = automatic_update_delay(self.settings.last_update_check, now, fallback_delay_ms);
        self.reset_update_timer(delay);
    }

    fn reset_update_timer(&self, delay_ms: u32) {
        unsafe {
            let _ = KillTimer(Some(self.hwnd), TIMER_UPDATE);
            let _ = SetTimer(Some(self.hwnd), TIMER_UPDATE, delay_ms.max(1_000), None);
        }
    }

    fn start_update_check(&mut self, kind: UpdateCheckKind) -> bool {
        if self.update_checking || self.pending_update.is_some() {
            return false;
        }
        if !updater::updates_supported() {
            if kind == UpdateCheckKind::Manual {
                self.show_balloon(
                    self.locale.text("Updates unavailable", "此构建不提供更新"),
                    self.locale.text(
                        "Development and beta builds do not install release-channel updates.",
                        "development 与 beta 构建不会安装发布 channel 的更新。",
                    ),
                );
            }
            return false;
        }
        let target = match std::env::current_exe() {
            Ok(target) => target,
            Err(error) => {
                match kind {
                    UpdateCheckKind::Automatic => self.reset_update_timer(UPDATE_RETRY_MS),
                    UpdateCheckKind::Manual => {
                        self.schedule_update_check(UPDATE_INITIAL_DELAY_MS);
                        self.show_update_check_failure(&error.to_string());
                    }
                }
                return false;
            }
        };
        if let Err(error) = updater::validate_target_for_update(&target) {
            if matches!(
                error,
                updater::UpdateError::UnsafeTarget | updater::UpdateError::TargetNotWritable
            ) {
                self.handle_unreplaceable_update_target(kind, &target, &error);
            } else {
                match kind {
                    UpdateCheckKind::Automatic => self.reset_update_timer(UPDATE_RETRY_MS),
                    UpdateCheckKind::Manual => {
                        self.schedule_update_check(UPDATE_INITIAL_DELAY_MS);
                        self.show_update_check_failure(&error.to_string());
                    }
                }
            }
            return false;
        }
        if kind.bypasses_daily_throttle() {
            unsafe {
                let _ = KillTimer(Some(self.hwnd), TIMER_UPDATE);
            }
        }
        self.update_checking = true;
        let hwnd_value = self.hwnd.0 as isize;
        let updates_directory = self.store.updates_directory();
        let spawn_result = thread::Builder::new()
            .name("codex-status-update".to_owned())
            .stack_size(512 * 1024)
            .spawn(move || {
                let hwnd = HWND(hwnd_value as *mut std::ffi::c_void);
                let outcome =
                    UpdateOutcome { kind, result: updater::check_and_stage(&updates_directory) };
                let raw = Box::into_raw(Box::new(outcome));
                let posted = unsafe {
                    PostMessageW(Some(hwnd), WM_UPDATE_COMPLETE, WPARAM(0), LPARAM(raw as isize))
                };
                if posted.is_err() {
                    unsafe {
                        drop(Box::from_raw(raw));
                    }
                }
            });
        if let Err(error) = spawn_result {
            self.update_checking = false;
            match kind {
                UpdateCheckKind::Automatic => self.reset_update_timer(UPDATE_RETRY_MS),
                UpdateCheckKind::Manual => {
                    self.schedule_update_check(UPDATE_INITIAL_DELAY_MS);
                    self.show_update_check_failure(&error.to_string());
                }
            }
            return false;
        }
        true
    }

    fn handle_unreplaceable_update_target(
        &mut self,
        kind: UpdateCheckKind,
        target: &Path,
        error: &updater::UpdateError,
    ) {
        let target_key = update_target_key(target);
        let should_notify = should_show_update_target_warning(
            kind,
            &self.settings.unreplaceable_update_targets,
            &target_key,
        );
        let warning_changed = !self.settings.unreplaceable_update_targets.contains(&target_key);
        if warning_changed || kind.records_last_check() {
            let now = Utc::now().timestamp();
            self.update_settings(|settings| {
                if warning_changed {
                    settings.unreplaceable_update_targets.push(target_key);
                }
                if kind.records_last_check() {
                    settings.last_update_check = Some(now);
                }
            });
        }
        if kind == UpdateCheckKind::Automatic {
            self.reset_update_timer(UPDATE_INTERVAL_SECONDS as u32 * 1_000);
        }
        if !should_notify {
            return;
        }

        match error {
            updater::UpdateError::UnsafeTarget => self.show_action_required_balloon(
                self.locale.text("Automatic update unavailable", "无法自动更新"),
                self.locale.text(
                    "The executable name is not supported. Open Releases from the tray menu and download the latest version, or rename this file to CodexStatus.exe.",
                    "当前可执行文件名不受支持。请从托盘菜单打开发布页并手动下载最新版，或将此文件重命名为 CodexStatus.exe。",
                ),
            ),
            updater::UpdateError::TargetNotWritable => self.show_action_required_balloon(
                self.locale.text("Automatic update unavailable", "无法自动更新"),
                self.locale.text(
                    "CodexStatus cannot replace the executable in this location. Move it to a writable folder, or open Releases and download the latest version manually.",
                    "CodexStatus 无法替换当前位置的可执行文件。请将它移到可写文件夹，或打开发布页手动下载最新版。",
                ),
            ),
            _ => self.show_update_check_failure(&error.to_string()),
        }
    }

    fn finish_update_check(&mut self, outcome: UpdateOutcome) {
        self.update_checking = false;
        self.schedule_working_set_trim();
        debug_assert_eq!(
            outcome.kind.records_last_check(),
            outcome.kind == UpdateCheckKind::Automatic
        );
        match outcome.kind {
            UpdateCheckKind::Automatic => match outcome.result {
                Ok(update) => {
                    self.settings.last_update_check = Some(Utc::now().timestamp());
                    self.persist_settings();
                    self.pending_update = update;
                    self.pending_update_manual = false;
                    if self.pending_update.is_some() {
                        self.try_apply_update();
                    } else {
                        self.reset_update_timer(UPDATE_INTERVAL_SECONDS as u32 * 1_000);
                    }
                }
                Err(_) => self.reset_update_timer(UPDATE_RETRY_MS),
            },
            UpdateCheckKind::Manual => {
                self.schedule_update_check(UPDATE_INITIAL_DELAY_MS);
                match outcome.result {
                    Ok(Some(update)) => {
                        #[cfg(feature = "diagnostics")]
                        diagnostic_event(
                            "manual_update_check",
                            serde_json::json!({ "result": "update_found" }),
                        );
                        self.show_balloon(
                            self.locale.text("Update found", "发现新版本"),
                            self.locale.text(
                                "The update is ready. CodexStatus will restart to install it.",
                                "更新已准备好，CodexStatus 将重启并完成安装。",
                            ),
                        );
                        self.pending_update = Some(update);
                        self.pending_update_manual = true;
                        self.try_apply_update();
                    }
                    Ok(None) => {
                        #[cfg(feature = "diagnostics")]
                        diagnostic_event(
                            "manual_update_check",
                            serde_json::json!({ "result": "up_to_date" }),
                        );
                        self.show_balloon(
                            self.locale
                                .text("CodexStatus is up to date", "CodexStatus 已是最新版本"),
                            &format!(
                                "{} {}",
                                self.locale.text("Current version", "当前版本"),
                                env!("CARGO_PKG_VERSION")
                            ),
                        );
                    }
                    Err(error) => {
                        #[cfg(feature = "diagnostics")]
                        diagnostic_event(
                            "manual_update_check",
                            serde_json::json!({
                                "result": "failed",
                                "error": error.to_string(),
                            }),
                        );
                        self.show_update_check_failure(&error.to_string());
                    }
                }
            }
        }
    }

    fn try_apply_update(&mut self) {
        if unsafe { IsWindowVisible(self.flyout) }.as_bool() {
            self.reset_update_timer(60_000);
            return;
        }
        let Some(update) = self.pending_update.take() else {
            return;
        };
        let manual = self.pending_update_manual;
        self.pending_update_manual = false;
        if updater::launch_staged_update(&update).is_ok() {
            unsafe {
                let _ = DestroyWindow(self.hwnd);
            }
        } else {
            self.reset_update_timer(UPDATE_RETRY_MS);
            if manual {
                self.show_update_check_failure(
                    self.locale.text(
                        "The verified update could not be started.",
                        "已验证的更新无法启动。",
                    ),
                );
            }
        }
    }

    fn start_manual_update_check(&mut self) {
        if self.update_checking {
            self.show_balloon(
                self.locale.text("Update check in progress", "正在检查更新"),
                self.locale
                    .text("Please wait for the current check to finish.", "请等待当前检查完成。"),
            );
            return;
        }
        if self.pending_update.is_some() {
            self.pending_update_manual = true;
            self.show_balloon(
                self.locale.text("Update ready", "更新已准备好"),
                self.locale.text(
                    "CodexStatus will restart to install the verified update.",
                    "CodexStatus 将重启并安装已验证的更新。",
                ),
            );
            self.try_apply_update();
            return;
        }
        if self.start_update_check(UpdateCheckKind::Manual) {
            #[cfg(feature = "diagnostics")]
            diagnostic_event("manual_update_check", serde_json::json!({ "result": "started" }));
            self.show_balloon(
                self.locale.text("Checking for updates", "正在检查更新"),
                self.locale.text(
                    "CodexStatus is checking the latest release.",
                    "CodexStatus 正在检查最新发布版本。",
                ),
            );
        }
    }

    fn show_update_check_failure(&self, error: &str) {
        self.show_balloon(
            self.locale.text("Update check failed", "检查更新失败"),
            &format!("{}: {error}", self.locale.text("Reason", "原因")),
        );
    }

    fn schedule_working_set_trim(&self) {
        unsafe {
            let _ = KillTimer(Some(self.hwnd), TIMER_WORKING_SET_TRIM);
            let _ =
                SetTimer(Some(self.hwnd), TIMER_WORKING_SET_TRIM, UPDATE_WORKING_SET_TRIM_MS, None);
        }
    }

    fn trim_working_set(&self) {
        unsafe {
            let _ = KillTimer(Some(self.hwnd), TIMER_WORKING_SET_TRIM);
            if IsWindowVisible(self.flyout).as_bool() {
                let _ = SetTimer(Some(self.hwnd), TIMER_WORKING_SET_TRIM, 30_000, None);
                return;
            }
            let _ = EmptyWorkingSet(GetCurrentProcess());
        }
    }

    fn start_refresh(&mut self, force: bool) {
        diagnostic("refresh:start");
        let Some(refresh_id) = self.refresh_sequence.begin(force, self.refresh_paused) else {
            return;
        };
        unsafe {
            let _ = KillTimer(Some(self.hwnd), TIMER_REFRESH_WATCHDOG);
            let _ = SetTimer(Some(self.hwnd), TIMER_REFRESH_WATCHDOG, REFRESH_WATCHDOG_MS, None);
        }
        self.start_refresh_animation();
        self.display.error = None;
        if self.display.snapshot.is_none() {
            self.display.refresh_state = RefreshState::Loading;
        }
        let _ = self.update_tray(false);
        unsafe {
            let _ = InvalidateRect(Some(self.flyout), None, false);
        }

        let hwnd_value = self.hwnd.0 as isize;
        let client = self.client.clone();
        let spawn_result = thread::Builder::new()
            .name("codex-status-refresh".to_owned())
            .stack_size(512 * 1024)
            .spawn(move || {
                let hwnd = HWND(hwnd_value as *mut std::ffi::c_void);
                diagnostic("refresh:worker");
                #[cfg(feature = "diagnostics")]
                if let Some(milliseconds) =
                    std::env::var("CODEX_STATUS_DIAGNOSTIC_REFRESH_STALL_MS")
                        .ok()
                        .and_then(|value| value.parse::<u64>().ok())
                {
                    thread::sleep(Duration::from_millis(milliseconds.min(60_000)));
                }
                let outcome = RefreshOutcome {
                    id: refresh_id,
                    result: client.fetch().map_err(|error| error.to_string()),
                };
                diagnostic(if outcome.result.is_ok() {
                    "refresh:success"
                } else {
                    "refresh:error"
                });
                let raw = Box::into_raw(Box::new(outcome));
                let posted = unsafe {
                    PostMessageW(Some(hwnd), WM_REFRESH_COMPLETE, WPARAM(0), LPARAM(raw as isize))
                };
                if posted.is_err() {
                    unsafe {
                        drop(Box::from_raw(raw));
                    }
                }
            });
        if let Err(error) = spawn_result {
            self.finish_refresh(RefreshOutcome {
                id: refresh_id,
                result: Err(format!("Could not start refresh: {error}")),
            });
        }
    }

    fn finish_refresh(&mut self, outcome: RefreshOutcome) {
        let Some(restart) = self.refresh_sequence.finish(outcome.id) else {
            #[cfg(feature = "diagnostics")]
            diagnostic_event("refresh_stale_completion", serde_json::json!({ "id": outcome.id }));
            return;
        };
        unsafe {
            let _ = KillTimer(Some(self.hwnd), TIMER_REFRESH_WATCHDOG);
        }
        self.stop_refresh_animation();
        match outcome.result {
            Ok(snapshot) => {
                self.failures = 0;
                let _ = self.store.save_snapshot(&snapshot);
                self.display = DisplayState::live(snapshot);
                self.reset_refresh_timer(self.settings.refresh_minutes.saturating_mul(60_000));
                self.maybe_alerts();
            }
            Err(error) => {
                self.failures = self.failures.saturating_add(1);
                let now = Utc::now().timestamp();
                let snapshot = self.display.snapshot.take();
                self.display =
                    DisplayState::after_error(snapshot, friendly_error(&error, self.locale), now);
                let backoff = match self.failures {
                    1 => 60_000,
                    2 => 5 * 60_000,
                    _ => 15 * 60_000,
                };
                self.reset_refresh_timer(backoff);
            }
        }
        let _ = self.update_tray(false);
        unsafe {
            let _ = InvalidateRect(Some(self.flyout), None, false);
        }
        if restart {
            self.start_refresh(true);
        }
    }

    fn expire_cache_if_needed(&mut self) {
        if self.display.refresh_state == RefreshState::Live {
            return;
        }
        let now = Utc::now().timestamp();
        if self.display.snapshot.as_ref().is_some_and(|value| !value.is_cache_valid(now)) {
            self.display.snapshot = None;
            self.display.refresh_state = RefreshState::Unavailable;
            let _ = self.update_tray(false);
        }
    }

    fn reset_refresh_timer(&self, milliseconds: u32) {
        unsafe {
            let _ = KillTimer(Some(self.hwnd), TIMER_REFRESH);
            if !self.refresh_paused {
                let interval = milliseconds.max(1_000);
                let timer = SetTimer(Some(self.hwnd), TIMER_REFRESH, interval, None);
                #[cfg(feature = "diagnostics")]
                diagnostic_event(
                    "timer",
                    serde_json::json!({
                        "kind": "refresh",
                        "active": timer != 0,
                        "interval_ms": interval,
                    }),
                );
                #[cfg(not(feature = "diagnostics"))]
                let _ = timer;
            }
        }
    }

    fn update_tray(&mut self, force_add: bool) -> windows::core::Result<()> {
        diagnostic("tray:render");
        let dpi = unsafe { GetDpiForSystem().max(96) };
        let size = unsafe { GetSystemMetricsForDpi(SM_CXSMICON, dpi).max(16) as u32 };
        let percent = ui::tray_percent(&self.display, &self.settings.tray_metric);
        let service_overlay =
            if self.settings.service_status_checks && self.service_status.codex_affected {
                match self.service_status.status {
                    ServiceStatus::Degraded => ServiceOverlay::Degraded,
                    ServiceStatus::Outage => ServiceOverlay::Outage,
                    _ => ServiceOverlay::None,
                }
            } else {
                ServiceOverlay::None
            };
        let tone = tone_for_percent(&self.display, percent);
        #[cfg(feature = "diagnostics")]
        let icon_digest = {
            let pixels = render_bgra_with_overlay(
                percent,
                tone,
                service_overlay,
                size,
                self.theme.high_contrast,
                self.theme.tray_dark,
            );
            format!("{:x}", Sha256::digest(pixels))
        };
        let icon = create_icon_with_overlay(
            percent,
            tone,
            service_overlay,
            size,
            self.theme.high_contrast,
            self.theme.tray_dark,
        )?;
        let mut data = self.notify_data();
        data.uFlags |= NIF_MESSAGE | NIF_ICON | NIF_TIP | NIF_SHOWTIP;
        data.uCallbackMessage = WM_TRAY;
        data.hIcon = icon.handle();
        let mut tooltip =
            ui::tooltip_for_metric(&self.display, self.locale, &self.settings.tray_metric);
        if self.service_status.codex_affected {
            tooltip.push_str(self.locale.text(" · Codex service incident", " · Codex 服务异常"));
        }
        copy_utf16(&mut data.szTip, &tooltip);
        let add = force_add || !self.tray_added;
        let operation = if add { NIM_ADD } else { NIM_MODIFY };
        diagnostic(if add { "tray:add" } else { "tray:modify" });
        let succeeded = unsafe { Shell_NotifyIconW(operation, &data) }.as_bool();
        #[cfg(feature = "diagnostics")]
        diagnostic_event(
            "tray_update",
            serde_json::json!({
                "operation": if add { "add" } else { "modify" },
                "success": succeeded,
                "metric": self.settings.tray_metric,
                "percent": percent,
                "icon_sha256": icon_digest,
                "tooltip": tooltip,
            }),
        );
        if !succeeded {
            diagnostic("tray:failed");
            return Err(windows::core::Error::from_thread());
        }
        diagnostic("tray:ok");
        self.tray_icon = Some(icon);
        if add {
            self.tray_added = true;
            let mut version = self.notify_data();
            version.Anonymous.uVersion = NOTIFYICON_VERSION_4;
            let _ = unsafe { Shell_NotifyIconW(NIM_SETVERSION, &version) };
            if !self.settings.onboarding_shown {
                self.show_balloon(
                    self.locale.text("CodexStatus is ready", "CodexStatus 已就绪"),
                    self.locale.text(
                        "Your weekly quota is shown in the tray. Drag the icon out of the overflow area to keep it visible.",
                        "周剩余额度会直接显示在托盘图标中。可将图标从折叠区拖出，保持常显。",
                    ),
                );
                self.settings.onboarding_shown = true;
                let _ = self.store.save_settings(&self.settings);
            }
        }
        Ok(())
    }

    fn notify_data(&self) -> NOTIFYICONDATAW {
        #[cfg(codex_status_channel = "portable")]
        {
            NOTIFYICONDATAW {
                cbSize: size_of::<NOTIFYICONDATAW>() as u32,
                hWnd: self.hwnd,
                uID: TRAY_ID,
                ..Default::default()
            }
        }
        #[cfg(not(codex_status_channel = "portable"))]
        {
            NOTIFYICONDATAW {
                cbSize: size_of::<NOTIFYICONDATAW>() as u32,
                hWnd: self.hwnd,
                uID: TRAY_ID,
                guidItem: TRAY_GUID,
                uFlags: NIF_GUID,
                ..Default::default()
            }
        }
    }

    fn show_balloon(&self, title: &str, body: &str) {
        let _ = self.submit_balloon(title, body, NotificationKind::Alert);
    }

    fn show_action_required_balloon(&self, title: &str, body: &str) {
        let _ = self.submit_balloon(title, body, NotificationKind::ActionRequired);
    }

    fn submit_balloon(&self, title: &str, body: &str, kind: NotificationKind) -> bool {
        let respect_quiet_time = kind.respects_quiet_time();
        if !self.tray_added {
            #[cfg(feature = "diagnostics")]
            diagnostic_event(
                "notification",
                serde_json::json!({
                    "title": title,
                    "body": body,
                    "submitted": false,
                    "reason": "tray_not_added",
                    "respect_quiet_time": respect_quiet_time,
                }),
            );
            return false;
        }
        let mut data = self.notify_data();
        data.uFlags |= NIF_INFO;
        data.dwInfoFlags =
            if respect_quiet_time { NIIF_INFO | NIIF_RESPECT_QUIET_TIME } else { NIIF_INFO };
        copy_utf16(&mut data.szInfoTitle, title);
        copy_utf16(&mut data.szInfo, body);
        let submitted = unsafe { Shell_NotifyIconW(NIM_MODIFY, &data) }.as_bool();
        #[cfg(feature = "diagnostics")]
        diagnostic_event(
            "notification",
            serde_json::json!({
                "title": title,
                "body": body,
                "submitted": submitted,
                "respect_quiet_time": respect_quiet_time,
            }),
        );
        submitted
    }

    fn show_test_notification(&mut self) {
        self.test_notification_pending = true;
        let submitted = self.submit_balloon(
            self.locale.text("CodexStatus notification", "CodexStatus 通知"),
            self.locale.text(
                "Notifications are working. No quota setting was changed.",
                "通知工作正常，未修改任何额度设置。",
            ),
            NotificationKind::Test,
        );
        if !submitted {
            self.test_notification_pending = false;
            self.show_test_notification_fallback();
            return;
        }
        let timer = unsafe {
            SetTimer(
                Some(self.hwnd),
                TIMER_TEST_NOTIFICATION_FEEDBACK,
                TEST_NOTIFICATION_FEEDBACK_MS,
                None,
            )
        };
        if timer == 0 {
            self.test_notification_pending = false;
            self.show_test_notification_fallback();
        }
    }

    fn show_test_notification_fallback(&self) {
        #[cfg(feature = "diagnostics")]
        diagnostic_event(
            "test_notification_feedback",
            serde_json::json!({ "displayed": false, "fallback": true }),
        );
        let title = wide0(self.locale.text("Test notification was not shown", "测试通知未显示"));
        let body = wide0(self.locale.text(
            "Windows did not display the notification. Check system notification settings or policy.",
            "Windows 未显示通知，请检查系统通知设置或策略。",
        ));
        unsafe {
            let _ = MessageBoxW(
                Some(self.hwnd),
                PCWSTR(body.as_ptr()),
                PCWSTR(title.as_ptr()),
                MESSAGEBOX_STYLE(MB_OK.0 | MB_ICONWARNING.0),
            );
        }
    }

    fn maybe_alerts(&mut self) {
        let Some(snapshot) = self.display.snapshot.clone() else {
            return;
        };
        let now = Utc::now().timestamp();
        let mut attention = Vec::new();
        let mut recovered = Vec::new();
        let mut settings_changed = false;

        for (kind, window, threshold) in [
            (QuotaKind::Weekly, snapshot.weekly.as_ref(), self.settings.alert_threshold),
            (QuotaKind::Session, snapshot.session.as_ref(), self.settings.session_alert_threshold),
        ] {
            let decision = evaluate_alerts(self.alert_tracker, kind, window, threshold, now);
            self.alert_tracker = decision.tracker;
            let label = quota_kind_label(kind, self.locale);

            if decision.should_notify_low {
                if let Some(window) = window {
                    attention.push(format!(
                        "{label}: {}% {}",
                        window.display_percent(),
                        self.locale.text("remaining", "剩余")
                    ));
                    let reset = window.resets_at;
                    match kind {
                        QuotaKind::Weekly => self.settings.last_alert_reset = reset,
                        QuotaKind::Session => self.settings.last_session_alert_reset = reset,
                    }
                    settings_changed = true;
                }
            }
            if self.settings.recovery_alerts && decision.should_notify_recovered {
                recovered.push(format!(
                    "{label} {}",
                    self.locale.text("is available again", "已恢复可用")
                ));
            }

            if self.settings.pace_alerts {
                if let Some(window) = window {
                    let insight = analyze_window(window, now);
                    let last_reset = match kind {
                        QuotaKind::Weekly => self.settings.last_weekly_pace_alert_reset,
                        QuotaKind::Session => self.settings.last_session_pace_alert_reset,
                    };
                    if insight.is_ahead_of_pace
                        && insight.likely_exhaust_before_reset
                        && window.resets_at != last_reset
                    {
                        let projected = insight
                            .projected_exhaustion_at
                            .map(|timestamp| format_local_time(timestamp, self.locale))
                            .unwrap_or_else(|| {
                                self.locale.text("before reset", "重置前").to_owned()
                            });
                        attention.push(format!(
                            "{label}: {} {projected}",
                            self.locale.text("may run out by", "预计耗尽于")
                        ));
                        match kind {
                            QuotaKind::Weekly => {
                                self.settings.last_weekly_pace_alert_reset = window.resets_at
                            }
                            QuotaKind::Session => {
                                self.settings.last_session_pace_alert_reset = window.resets_at
                            }
                        }
                        settings_changed = true;
                    }
                }
            }
        }

        if !attention.is_empty() {
            self.show_balloon(
                self.locale.text("Codex quota needs attention", "Codex 额度需要留意"),
                &attention.join("\n"),
            );
        }
        if !recovered.is_empty() {
            self.show_balloon(
                self.locale.text("Codex quota recovered", "Codex 额度已恢复"),
                &recovered.join("\n"),
            );
        }
        if settings_changed {
            self.persist_settings();
        }
    }

    fn request_toggle_flyout(&mut self) {
        let now = Instant::now();
        if self
            .flyout_hidden_for_tray_activation
            .take()
            .is_some_and(|hidden| now.duration_since(hidden) < TRAY_CLOSE_COALESCE)
        {
            // Clicking the icon transfers focus to Explorer before its tray
            // callback arrives. If that focus loss already hid the card, the
            // callback represents the same click and must not reopen it.
            self.last_tray_activation = Some(now);
            return;
        }
        if self
            .last_tray_activation
            .is_some_and(|previous| now.duration_since(previous) < TRAY_ACTIVATION_DEBOUNCE)
        {
            return;
        }
        self.last_tray_activation = Some(now);
        unsafe {
            // Showing from inside the Explorer callback lets the shell reclaim
            // activation and used to make the card flash closed. Defer it until
            // the notification callback has completely returned.
            let _ = PostMessageW(Some(self.hwnd), WM_TOGGLE_FLYOUT, WPARAM(0), LPARAM(0));
        }
    }

    fn toggle_flyout(&mut self) {
        if unsafe { IsWindowVisible(self.flyout) }.as_bool() {
            self.hide_flyout();
        } else {
            self.show_flyout();
        }
    }

    fn hide_flyout(&mut self) {
        self.flyout_ignore_inactive_until = None;
        self.flyout_hidden_for_tray_activation = None;
        self.refresh_hovered = false;
        self.refresh_pointer_down = false;
        self.stop_refresh_animation();
        unsafe {
            let _ = ReleaseCapture();
            let _ = KillTimer(Some(self.hwnd), TIMER_FLYOUT_ACTIVATE);
            let _ = KillTimer(Some(self.hwnd), TIMER_CARD);
            let _ = KillTimer(Some(self.hwnd), TIMER_RENDERER_RELEASE);
            let _ = ShowWindow(self.flyout, SW_HIDE);
        }
        ui::release_card_surface();
        unsafe {
            let _ =
                SetTimer(Some(self.hwnd), TIMER_RENDERER_RELEASE, RENDERER_IDLE_RELEASE_MS, None);
        }
        self.schedule_working_set_trim();
        self.try_apply_update();
    }

    fn handle_flyout_inactive(&mut self) {
        if self.settings.flyout_pinned {
            self.flyout_ignore_inactive_until = None;
            return;
        }
        let guarded =
            self.flyout_ignore_inactive_until.is_some_and(|deadline| Instant::now() < deadline);
        if guarded {
            unsafe {
                let _ = KillTimer(Some(self.hwnd), TIMER_FLYOUT_ACTIVATE);
                let _ = SetTimer(Some(self.hwnd), TIMER_FLYOUT_ACTIVATE, 75, None);
            }
        } else {
            let over_tray_icon = self.cursor_is_over_tray_icon();
            self.hide_flyout();
            if over_tray_icon {
                self.flyout_hidden_for_tray_activation = Some(Instant::now());
            }
        }
    }

    fn finish_flyout_activation(&mut self) {
        if !unsafe { IsWindowVisible(self.flyout) }.as_bool() {
            self.flyout_ignore_inactive_until = None;
            return;
        }
        unsafe {
            let _ = SetForegroundWindow(self.flyout);
        }
        self.flyout_ignore_inactive_until = None;
    }

    fn cursor_is_over_tray_icon(&self) -> bool {
        let Some(rect) = self.tray_rect() else {
            return false;
        };
        let mut point = POINT::default();
        unsafe {
            let _ = GetCursorPos(&mut point);
        }
        point.x >= rect.left && point.x < rect.right && point.y >= rect.top && point.y < rect.bottom
    }

    fn show_flyout(&mut self) {
        self.flyout_hidden_for_tray_activation = None;
        self.theme = ui::detect_theme(&self.settings.theme);
        ui::configure_flyout(self.flyout, self.theme);
        let anchor = self.tray_rect().unwrap_or_else(cursor_rect);
        let point =
            POINT { x: (anchor.left + anchor.right) / 2, y: (anchor.top + anchor.bottom) / 2 };
        let monitor = unsafe { MonitorFromPoint(point, MONITOR_DEFAULTTONEAREST) };
        let mut info =
            MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
        unsafe {
            let _ = GetMonitorInfoW(monitor, &mut info);
        }
        let dpi = unsafe {
            let mut dpi_x = 96;
            let mut dpi_y = 96;
            if GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y).is_ok() {
                dpi_x.max(96)
            } else {
                // This fallback is only used if shcore cannot query the target
                // monitor. Never raise it to the system DPI: a PMv2 window may
                // legitimately reside on a lower-DPI monitor.
                GetDpiForWindow(self.flyout).max(96)
            }
        };
        let width = ui::scale(ui::CARD_WIDTH, dpi);
        let height = ui::scale(ui::CARD_HEIGHT, dpi);
        let gap = ui::scale(10, dpi);
        let mut x = point.x - width / 2;
        let mut y = anchor.top - height - gap;
        if y < info.rcWork.top {
            y = anchor.bottom + gap;
        }
        x = x.clamp(info.rcWork.left, (info.rcWork.right - width).max(info.rcWork.left));
        y = y.clamp(info.rcWork.top, (info.rcWork.bottom - height).max(info.rcWork.top));
        self.flyout_ignore_inactive_until = Some(Instant::now() + FLYOUT_ACTIVATION_GUARD);
        unsafe {
            let _ = KillTimer(Some(self.hwnd), TIMER_FLYOUT_ACTIVATE);
            let _ = KillTimer(Some(self.hwnd), TIMER_RENDERER_RELEASE);
            let _ = SetWindowPos(self.flyout, None, x, y, width, height, SWP_SHOWWINDOW);
            let _ = SetForegroundWindow(self.flyout);
            let _ = SetTimer(Some(self.hwnd), TIMER_CARD, 30_000, None);
            let _ = InvalidateRect(Some(self.flyout), None, false);
        }
        self.start_refresh_animation();
    }

    fn start_refresh_animation(&mut self) {
        if !self.refresh_sequence.is_active()
            || self.refresh_paused
            || !unsafe { IsWindowVisible(self.flyout) }.as_bool()
        {
            return;
        }
        unsafe {
            let _ = KillTimer(Some(self.hwnd), TIMER_REFRESH_ANIMATION);
            if !self.refresh_timer_resolution_active && timeBeginPeriod(1) == TIMERR_NOERROR {
                self.refresh_timer_resolution_active = true;
            }
            let timer = SetTimer(
                Some(self.hwnd),
                TIMER_REFRESH_ANIMATION,
                REFRESH_ANIMATION_TIMER_MS,
                None,
            );
            #[cfg(feature = "diagnostics")]
            if timer != 0 {
                self.refresh_animation_started = Some(Instant::now());
                self.refresh_animation_frames = 0;
            }
            #[cfg(not(feature = "diagnostics"))]
            let _ = timer;
            if timer == 0 && self.refresh_timer_resolution_active {
                let _ = timeEndPeriod(1);
                self.refresh_timer_resolution_active = false;
            }
        }
    }

    fn stop_refresh_animation(&mut self) {
        #[cfg(feature = "diagnostics")]
        if let Some(started) = self.refresh_animation_started.take() {
            let elapsed = started.elapsed();
            diagnostic_event(
                "refresh_animation_stopped",
                serde_json::json!({
                    "frames": self.refresh_animation_frames,
                    "elapsed_ms": elapsed.as_millis(),
                    "fps": if elapsed.is_zero() {
                        0.0
                    } else {
                        f64::from(self.refresh_animation_frames) / elapsed.as_secs_f64()
                    },
                    "target_frame_ms": REFRESH_ANIMATION_TARGET_FRAME_MS,
                    "timer_ms": REFRESH_ANIMATION_TIMER_MS,
                }),
            );
            self.refresh_animation_frames = 0;
        }
        unsafe {
            let _ = KillTimer(Some(self.hwnd), TIMER_REFRESH_ANIMATION);
            if self.refresh_timer_resolution_active {
                let _ = timeEndPeriod(1);
                self.refresh_timer_resolution_active = false;
            }
        }
        self.refresh_angle_degrees = 0;
    }

    fn tray_rect(&self) -> Option<RECT> {
        #[cfg(codex_status_channel = "portable")]
        let identifier = NOTIFYICONIDENTIFIER {
            cbSize: size_of::<NOTIFYICONIDENTIFIER>() as u32,
            hWnd: self.hwnd,
            uID: TRAY_ID,
            ..Default::default()
        };
        #[cfg(not(codex_status_channel = "portable"))]
        let identifier = NOTIFYICONIDENTIFIER {
            cbSize: size_of::<NOTIFYICONIDENTIFIER>() as u32,
            hWnd: self.hwnd,
            uID: TRAY_ID,
            guidItem: TRAY_GUID,
        };
        unsafe { Shell_NotifyIconGetRect(&identifier).ok() }
    }

    fn show_menu(&mut self) {
        let Ok(menu) = self.build_menu() else {
            return;
        };
        let mut point = POINT::default();
        unsafe {
            let _ = GetCursorPos(&mut point);
            let _ = SetForegroundWindow(self.hwnd);
            let command = TrackPopupMenu(
                menu,
                TPM_LEFTALIGN | TPM_BOTTOMALIGN | TPM_RIGHTBUTTON | TPM_RETURNCMD,
                point.x,
                point.y,
                None,
                self.hwnd,
                None,
            )
            .0 as u32;
            let _ = DestroyMenu(menu);
            let _ = PostMessageW(Some(self.hwnd), WM_NULL, WPARAM(0), LPARAM(0));
            self.handle_command(command);
        }
    }

    fn build_menu(&self) -> windows::core::Result<HMENU> {
        let menu = unsafe { CreatePopupMenu()? };
        let startup_enabled = startup::is_enabled();
        unsafe {
            append(menu, CMD_REFRESH, self.locale.text("Refresh now", "立即刷新"), false)?;
            append(
                menu,
                CMD_USAGE,
                self.locale.text("Open Codex usage", "打开 Codex 用量页"),
                false,
            )?;
            separator(menu)?;

            let interval_menu = CreatePopupMenu()?;
            append(
                interval_menu,
                CMD_INTERVAL_1,
                self.locale.text("Every 1 minute", "每 1 分钟"),
                self.settings.refresh_minutes == 1,
            )?;
            append(
                interval_menu,
                CMD_INTERVAL_5,
                self.locale.text("Every 5 minutes", "每 5 分钟"),
                self.settings.refresh_minutes == 5,
            )?;
            append(
                interval_menu,
                CMD_INTERVAL_15,
                self.locale.text("Every 15 minutes", "每 15 分钟"),
                self.settings.refresh_minutes == 15,
            )?;
            let selected_interval = match self.settings.refresh_minutes {
                1 => CMD_INTERVAL_1,
                15 => CMD_INTERVAL_15,
                _ => CMD_INTERVAL_5,
            };
            radio_group(interval_menu, CMD_INTERVAL_1, CMD_INTERVAL_15, selected_interval)?;
            append_submenu(menu, interval_menu, self.locale.text("Refresh interval", "刷新间隔"))?;

            let tray_menu = CreatePopupMenu()?;
            append(
                tray_menu,
                CMD_TRAY_WEEKLY,
                self.locale.text("Weekly quota", "周额度"),
                self.settings.tray_metric == "weekly",
            )?;
            append(
                tray_menu,
                CMD_TRAY_SESSION,
                self.locale.text("5-hour quota", "5 小时额度"),
                self.settings.tray_metric == "session",
            )?;
            append(
                tray_menu,
                CMD_TRAY_LOWEST,
                self.locale.text("Lower of both", "显示较低值"),
                self.settings.tray_metric == "lowest",
            )?;
            let selected_tray_metric = match self.settings.tray_metric.as_str() {
                "session" => CMD_TRAY_SESSION,
                "lowest" => CMD_TRAY_LOWEST,
                _ => CMD_TRAY_WEEKLY,
            };
            radio_group(tray_menu, CMD_TRAY_WEEKLY, CMD_TRAY_LOWEST, selected_tray_metric)?;
            append_submenu(menu, tray_menu, self.locale.text("Tray display", "托盘显示"))?;

            let alert_menu = CreatePopupMenu()?;
            append_disabled(alert_menu, self.locale.text("Weekly quota", "周额度"))?;
            append(
                alert_menu,
                CMD_ALERT_OFF,
                self.locale.text("Off", "关闭"),
                self.settings.alert_threshold.is_none(),
            )?;
            for (command, threshold) in [(CMD_ALERT_10, 10), (CMD_ALERT_20, 20), (CMD_ALERT_30, 30)]
            {
                let label = match (self.locale, threshold) {
                    (ui::Locale::Chinese, value) => format!("剩余不高于 {value}% 时提醒"),
                    (_, value) => format!("Alert at or below {value}%"),
                };
                append(
                    alert_menu,
                    command,
                    &label,
                    self.settings.alert_threshold == Some(threshold),
                )?;
            }
            let selected_weekly_alert = match self.settings.alert_threshold {
                Some(10) => CMD_ALERT_10,
                Some(20) => CMD_ALERT_20,
                Some(30) => CMD_ALERT_30,
                _ => CMD_ALERT_OFF,
            };
            radio_group(alert_menu, CMD_ALERT_OFF, CMD_ALERT_30, selected_weekly_alert)?;
            separator(alert_menu)?;
            append_disabled(alert_menu, self.locale.text("5-hour quota", "5 小时额度"))?;
            append(
                alert_menu,
                CMD_SESSION_ALERT_OFF,
                self.locale.text("Off", "关闭"),
                self.settings.session_alert_threshold.is_none(),
            )?;
            for (command, threshold) in
                [(CMD_SESSION_ALERT_10, 10), (CMD_SESSION_ALERT_20, 20), (CMD_SESSION_ALERT_30, 30)]
            {
                let label = match (self.locale, threshold) {
                    (ui::Locale::Chinese, value) => format!("剩余不高于 {value}% 时提醒"),
                    (_, value) => format!("Alert at or below {value}%"),
                };
                append(
                    alert_menu,
                    command,
                    &label,
                    self.settings.session_alert_threshold == Some(threshold),
                )?;
            }
            let selected_session_alert = match self.settings.session_alert_threshold {
                Some(10) => CMD_SESSION_ALERT_10,
                Some(20) => CMD_SESSION_ALERT_20,
                Some(30) => CMD_SESSION_ALERT_30,
                _ => CMD_SESSION_ALERT_OFF,
            };
            radio_group(
                alert_menu,
                CMD_SESSION_ALERT_OFF,
                CMD_SESSION_ALERT_30,
                selected_session_alert,
            )?;
            separator(alert_menu)?;
            append(
                alert_menu,
                CMD_PACE_ALERTS,
                self.locale.text("Smart pace alerts", "智能用量节奏提醒"),
                self.settings.pace_alerts,
            )?;
            append(
                alert_menu,
                CMD_RECOVERY_ALERTS,
                self.locale.text("Recovery alerts", "额度恢复提醒"),
                self.settings.recovery_alerts,
            )?;
            append(
                alert_menu,
                CMD_TEST_NOTIFICATION,
                self.locale.text("Test notification", "测试通知"),
                false,
            )?;
            append_submenu(menu, alert_menu, self.locale.text("Alerts", "提醒"))?;

            let appearance_menu = CreatePopupMenu()?;
            append(
                appearance_menu,
                CMD_THEME_SYSTEM,
                self.locale.text("Follow system", "跟随系统"),
                self.settings.theme == "system",
            )?;
            append(
                appearance_menu,
                CMD_THEME_LIGHT,
                self.locale.text("Light", "浅色"),
                self.settings.theme == "light",
            )?;
            append(
                appearance_menu,
                CMD_THEME_DARK,
                self.locale.text("Dark", "深色"),
                self.settings.theme == "dark",
            )?;
            let selected_theme = match self.settings.theme.as_str() {
                "light" => CMD_THEME_LIGHT,
                "dark" => CMD_THEME_DARK,
                _ => CMD_THEME_SYSTEM,
            };
            radio_group(appearance_menu, CMD_THEME_SYSTEM, CMD_THEME_DARK, selected_theme)?;
            separator(appearance_menu)?;
            append(
                appearance_menu,
                CMD_PIN_FLYOUT,
                self.locale.text("Keep details open", "保持详情卡片常开"),
                self.settings.flyout_pinned,
            )?;
            append_submenu(menu, appearance_menu, self.locale.text("Appearance", "外观"))?;

            separator(menu)?;
            append(
                menu,
                CMD_COPY_STATUS,
                self.locale.text("Copy quota status", "复制额度状态"),
                false,
            )?;
            append(
                menu,
                CMD_COPY_DIAGNOSTICS,
                self.locale.text("Copy diagnostics", "复制诊断信息"),
                false,
            )?;
            separator(menu)?;
            append(menu, CMD_STATUS_PAGE, &self.service_status_label(), false)?;
            append(
                menu,
                CMD_STATUS_CHECKS,
                self.locale.text("Check OpenAI service status", "检查 OpenAI 服务状态"),
                self.settings.service_status_checks,
            )?;
            append(
                menu,
                CMD_GLOBAL_HOTKEY,
                self.locale.text("Global shortcut: Ctrl+Alt+Q", "全局快捷键：Ctrl+Alt+Q"),
                self.settings.global_hotkey,
            )?;
            append(
                menu,
                CMD_STARTUP,
                self.locale.text("Start with Windows", "开机自动启动"),
                startup_enabled,
            )?;
            append(menu, CMD_RELEASES, self.locale.text("Open releases", "打开发布页"), false)?;
            if !updater::updates_supported() {
                append_command_disabled(
                    menu,
                    CMD_CHECK_UPDATES,
                    self.locale.text("Updates unavailable in this build", "此构建不提供自动更新"),
                )?;
            } else if self.update_checking || self.pending_update.is_some() {
                append_command_disabled(
                    menu,
                    CMD_CHECK_UPDATES,
                    self.locale.text("Checking for updates…", "正在检查更新…"),
                )?;
            } else {
                append(
                    menu,
                    CMD_CHECK_UPDATES,
                    self.locale.text("Check for updates now", "立即检查更新"),
                    false,
                )?;
            }
            separator(menu)?;
            append(
                menu,
                CMD_EXIT,
                self.locale.text("Exit CodexStatus", "退出 CodexStatus"),
                false,
            )?;
        }
        Ok(menu)
    }

    #[cfg(feature = "diagnostics")]
    fn dump_menu_diagnostics(&self) {
        match self.build_menu() {
            Ok(menu) => unsafe {
                diagnostic_menu_items(menu);
                let _ = DestroyMenu(menu);
            },
            Err(error) => diagnostic_event(
                "menu_dump_failed",
                serde_json::json!({ "error": error.to_string() }),
            ),
        }
    }

    unsafe fn handle_command(&mut self, command: u32) {
        #[cfg(feature = "diagnostics")]
        diagnostic_event(
            "command_dispatch",
            serde_json::json!({
                "command": command,
                "settings": self.settings,
                "hotkey_registered": self.hotkey.is_some(),
            }),
        );
        unsafe {
            match command {
                0 => {}
                CMD_REFRESH => self.start_refresh(true),
                CMD_USAGE => self.open_url(USAGE_URL),
                CMD_COPY_STATUS => self.copy_to_clipboard(&self.status_text()),
                CMD_COPY_DIAGNOSTICS => self.copy_to_clipboard(&self.diagnostics_text()),
                CMD_STATUS_PAGE => self.open_url(STATUS_PAGE_HOME),
                CMD_RELEASES => self.open_url(RELEASES_URL),
                CMD_CHECK_UPDATES => self.start_manual_update_check(),
                CMD_INTERVAL_1 | CMD_INTERVAL_5 | CMD_INTERVAL_15 => {
                    let minutes = match command {
                        CMD_INTERVAL_1 => 1,
                        CMD_INTERVAL_15 => 15,
                        _ => 5,
                    };
                    if self.update_settings(|settings| settings.refresh_minutes = minutes) {
                        self.reset_refresh_timer(minutes * 60_000);
                    }
                }
                CMD_ALERT_OFF | CMD_ALERT_10 | CMD_ALERT_20 | CMD_ALERT_30 => {
                    let threshold = match command {
                        CMD_ALERT_10 => Some(10),
                        CMD_ALERT_20 => Some(20),
                        CMD_ALERT_30 => Some(30),
                        _ => None,
                    };
                    let mut changed = self.settings.clone();
                    if set_alert_threshold(&mut changed, QuotaKind::Weekly, threshold)
                        && self.update_settings(|settings| *settings = changed)
                    {
                        self.alert_tracker.weekly.low_alerted_cycle = None;
                        self.maybe_alerts();
                    }
                }
                CMD_SESSION_ALERT_OFF
                | CMD_SESSION_ALERT_10
                | CMD_SESSION_ALERT_20
                | CMD_SESSION_ALERT_30 => {
                    let threshold = match command {
                        CMD_SESSION_ALERT_10 => Some(10),
                        CMD_SESSION_ALERT_20 => Some(20),
                        CMD_SESSION_ALERT_30 => Some(30),
                        _ => None,
                    };
                    let mut changed = self.settings.clone();
                    if set_alert_threshold(&mut changed, QuotaKind::Session, threshold)
                        && self.update_settings(|settings| *settings = changed)
                    {
                        self.alert_tracker.session.low_alerted_cycle = None;
                        self.maybe_alerts();
                    }
                }
                CMD_PACE_ALERTS => {
                    let enabled = !self.settings.pace_alerts;
                    if self.update_settings(|settings| {
                        settings.pace_alerts = enabled;
                        settings.last_weekly_pace_alert_reset = None;
                        settings.last_session_pace_alert_reset = None;
                    }) {
                        self.maybe_alerts();
                    }
                }
                CMD_RECOVERY_ALERTS => {
                    let enabled = !self.settings.recovery_alerts;
                    self.update_settings(|settings| settings.recovery_alerts = enabled);
                }
                CMD_TEST_NOTIFICATION => self.show_test_notification(),
                CMD_TRAY_WEEKLY | CMD_TRAY_SESSION | CMD_TRAY_LOWEST => {
                    let metric = match command {
                        CMD_TRAY_SESSION => "session",
                        CMD_TRAY_LOWEST => "lowest",
                        _ => "weekly",
                    }
                    .to_owned();
                    if self.update_settings(|settings| settings.tray_metric = metric) {
                        self.update_tray_after_command();
                    }
                }
                CMD_STATUS_CHECKS => {
                    let enabled = !self.settings.service_status_checks;
                    if self.update_settings(|settings| settings.service_status_checks = enabled) {
                        if enabled {
                            self.reset_status_timer(1_000);
                        } else {
                            let stopped = KillTimer(Some(self.hwnd), TIMER_STATUS).is_ok();
                            #[cfg(feature = "diagnostics")]
                            diagnostic_event(
                                "timer",
                                serde_json::json!({
                                    "kind": "service_status",
                                    "active": false,
                                    "stop_succeeded": stopped,
                                }),
                            );
                            #[cfg(not(feature = "diagnostics"))]
                            let _ = stopped;
                            self.service_status = ServiceStatusSnapshot::unavailable();
                            self.update_tray_after_command();
                            let _ = InvalidateRect(Some(self.flyout), None, false);
                        }
                    }
                }
                CMD_GLOBAL_HOTKEY => self.toggle_global_hotkey(),
                CMD_PIN_FLYOUT => {
                    let pinned = !self.settings.flyout_pinned;
                    if self.update_settings(|settings| settings.flyout_pinned = pinned) {
                        let _ = InvalidateRect(Some(self.flyout), None, false);
                    }
                }
                CMD_STARTUP => {
                    let result = std::env::current_exe().and_then(|path| {
                        if startup::is_enabled_for(&path) {
                            startup::disable()
                        } else {
                            startup::enable(&path)
                        }
                    });
                    if let Err(error) = result {
                        self.show_balloon(
                            self.locale.text("Startup setting failed", "开机启动设置失败"),
                            &error.to_string(),
                        );
                    }
                }
                CMD_THEME_SYSTEM | CMD_THEME_LIGHT | CMD_THEME_DARK => {
                    let theme = match command {
                        CMD_THEME_LIGHT => "light",
                        CMD_THEME_DARK => "dark",
                        _ => "system",
                    }
                    .to_owned();
                    if self.update_settings(|settings| settings.theme = theme) {
                        self.theme = ui::detect_theme(&self.settings.theme);
                        ui::configure_flyout(self.flyout, self.theme);
                        self.update_tray_after_command();
                        let _ = InvalidateRect(Some(self.flyout), None, true);
                    }
                }
                CMD_EXIT => {
                    let _ = DestroyWindow(self.hwnd);
                }
                _ => {}
            }
        }
        #[cfg(feature = "diagnostics")]
        diagnostic_event(
            "command_complete",
            serde_json::json!({
                "command": command,
                "settings": self.settings,
                "hotkey_registered": self.hotkey.is_some(),
                "refreshing": self.refresh_sequence.is_active(),
                "tray_added": self.tray_added,
            }),
        );
    }

    fn persist_settings(&self) -> bool {
        let result = self.store.save_settings(&self.settings);
        #[cfg(feature = "diagnostics")]
        match &result {
            Ok(()) => diagnostic_event("settings_saved", serde_json::json!({ "success": true })),
            Err(error) => diagnostic_event(
                "settings_saved",
                serde_json::json!({ "success": false, "error": error.to_string() }),
            ),
        }
        #[cfg(not(feature = "diagnostics"))]
        let _ = &result;
        if let Err(error) = &result {
            self.show_balloon(
                self.locale.text("Settings not saved", "设置未保存"),
                &error.to_string(),
            );
        }
        result.is_ok()
    }

    fn update_settings(&mut self, update: impl FnOnce(&mut Settings)) -> bool {
        let result = save_settings_change(&self.store, &mut self.settings, update);
        #[cfg(feature = "diagnostics")]
        match &result {
            Ok(()) => diagnostic_event("settings_saved", serde_json::json!({ "success": true })),
            Err(error) => diagnostic_event(
                "settings_saved",
                serde_json::json!({ "success": false, "error": error.to_string() }),
            ),
        }
        if let Err(error) = &result {
            self.show_balloon(
                self.locale.text("Settings not saved", "设置未保存"),
                &error.to_string(),
            );
        }
        result.is_ok()
    }

    fn update_tray_after_command(&mut self) {
        if let Err(error) = self.update_tray(false) {
            self.show_balloon(
                self.locale.text("Tray update failed", "托盘更新失败"),
                &error.to_string(),
            );
        }
    }

    fn toggle_global_hotkey(&mut self) {
        if let Some(mut registration) = self.hotkey.take() {
            if let Err(error) = registration.unregister() {
                #[cfg(feature = "diagnostics")]
                diagnostic_event(
                    "hotkey_registration",
                    serde_json::json!({
                        "action": "unregister",
                        "success": false,
                        "error": error.to_string(),
                    }),
                );
                self.hotkey = Some(registration);
                self.show_balloon(
                    self.locale.text("Shortcut setting failed", "快捷键设置失败"),
                    &error.to_string(),
                );
                return;
            }
            #[cfg(feature = "diagnostics")]
            diagnostic_event(
                "hotkey_registration",
                serde_json::json!({ "action": "unregister", "success": true }),
            );
            if !self.update_settings(|settings| settings.global_hotkey = false) {
                match HotKeyRegistration::register(
                    Some(self.hwnd),
                    GLOBAL_HOTKEY_ID,
                    MOD_ALT | MOD_CONTROL | MOD_NOREPEAT,
                    u32::from(b'Q'),
                ) {
                    Ok(registration) => self.hotkey = Some(registration),
                    Err(error) => self.show_balloon(
                        self.locale.text("Shortcut setting failed", "快捷键设置失败"),
                        &error.to_string(),
                    ),
                }
            }
            return;
        }

        match HotKeyRegistration::register(
            Some(self.hwnd),
            GLOBAL_HOTKEY_ID,
            MOD_ALT | MOD_CONTROL | MOD_NOREPEAT,
            u32::from(b'Q'),
        ) {
            Ok(registration) => {
                #[cfg(feature = "diagnostics")]
                diagnostic_event(
                    "hotkey_registration",
                    serde_json::json!({ "action": "register", "success": true }),
                );
                if self.update_settings(|settings| settings.global_hotkey = true) {
                    self.hotkey = Some(registration);
                }
            }
            Err(error) => {
                #[cfg(feature = "diagnostics")]
                diagnostic_event(
                    "hotkey_registration",
                    serde_json::json!({
                        "action": "register",
                        "success": false,
                        "error": error.to_string(),
                    }),
                );
                #[cfg(not(feature = "diagnostics"))]
                let _ = &error;
                self.show_balloon(
                    self.locale.text("Shortcut unavailable", "快捷键不可用"),
                    self.locale.text(
                        "Ctrl+Alt+Q is already used by another app.",
                        "Ctrl+Alt+Q 已被其他应用占用。",
                    ),
                );
            }
        }
    }

    fn copy_to_clipboard(&self, text: &str) {
        let started = Instant::now();
        let result = write_unicode_text(self.hwnd, text);
        let elapsed_ms = started.elapsed().as_millis();
        match result {
            Ok(()) => {
                #[cfg(feature = "diagnostics")]
                diagnostic_event(
                    "clipboard",
                    serde_json::json!({
                        "success": true,
                        "text": text,
                        "elapsed_ms": elapsed_ms,
                    }),
                );
                #[cfg(not(feature = "diagnostics"))]
                let _ = elapsed_ms;
                self.show_balloon(
                    self.locale.text("Copied", "已复制"),
                    self.locale.text("Copied to the clipboard.", "内容已复制到剪贴板。"),
                );
            }
            Err(error) => {
                #[cfg(feature = "diagnostics")]
                diagnostic_event(
                    "clipboard",
                    serde_json::json!({
                        "success": false,
                        "error": error.to_string(),
                        "attempted_text": text,
                        "elapsed_ms": elapsed_ms,
                    }),
                );
                #[cfg(not(feature = "diagnostics"))]
                let _ = error;
                self.show_balloon(
                    self.locale.text("Could not copy", "复制失败"),
                    self.locale.text(
                        "The clipboard is busy. Please try again.",
                        "剪贴板正忙，请稍后重试。",
                    ),
                );
            }
        }
    }

    fn status_text(&self) -> String {
        let mut lines = vec!["CodexStatus".to_owned()];
        if let Some(snapshot) = self.display.snapshot.as_ref() {
            lines.push(quota_status_line(QuotaKind::Weekly, snapshot.weekly.as_ref(), self.locale));
            if snapshot.session.is_some() {
                lines.push(quota_status_line(
                    QuotaKind::Session,
                    snapshot.session.as_ref(),
                    self.locale,
                ));
            }
            if let Some(plan) = snapshot.account.plan_type.as_deref() {
                lines.push(format!(
                    "{}: {}",
                    self.locale.text("Plan", "套餐"),
                    ui::plan_label(plan, self.locale)
                ));
            }
            lines.push(format!(
                "{}: {}",
                self.locale.text("Updated", "更新时间"),
                format_local_time(snapshot.fetched_at, self.locale)
            ));
        } else {
            lines.push(self.locale.text("Quota unavailable", "额度暂不可用").to_owned());
        }
        lines.push(self.service_status_label());
        lines.join("\r\n")
    }

    fn diagnostics_text(&self) -> String {
        let snapshot = self.display.snapshot.as_ref();
        let weekly = snapshot.and_then(|value| value.weekly.as_ref());
        let session = snapshot.and_then(|value| value.session.as_ref());
        let error = self
            .display
            .error
            .as_deref()
            .map(|value| value.replace(['\r', '\n'], " "))
            .unwrap_or_else(|| "none".to_owned());
        format!(
            "CodexStatus diagnostics\r\nversion={}\r\nrefresh_state={:?}\r\nweekly={}\r\nsession={}\r\nlast_update={}\r\nrefresh_minutes={}\r\ntray_metric={}\r\ntheme={}\r\nservice_status={:?}\r\ncodex_affected={}\r\nservice_summary={}\r\nlast_error={}",
            env!("CARGO_PKG_VERSION"),
            self.display.refresh_state,
            diagnostic_window(weekly),
            diagnostic_window(session),
            snapshot.map(|value| value.fetched_at.to_string()).unwrap_or_else(|| "none".to_owned()),
            self.settings.refresh_minutes,
            self.settings.tray_metric,
            self.settings.theme,
            self.service_status.status,
            self.service_status.codex_affected,
            self.service_status.summary.replace(['\r', '\n'], " "),
            error,
        )
    }

    fn service_status_label(&self) -> String {
        if !self.settings.service_status_checks {
            return self
                .locale
                .text("OpenAI status: checks off", "OpenAI 状态：检查已关闭")
                .to_owned();
        }
        let status = match self.service_status.status {
            ServiceStatus::Operational => self.locale.text("Operational", "正常"),
            ServiceStatus::Degraded => self.locale.text("Degraded", "服务降级"),
            ServiceStatus::Outage => self.locale.text("Outage", "服务中断"),
            ServiceStatus::Unknown => self.locale.text("Unavailable", "暂不可用"),
        };
        format!("{}: {status}", self.locale.text("OpenAI status", "OpenAI 状态"))
    }

    fn open_url(&self, url: &str) {
        #[cfg(feature = "diagnostics")]
        {
            diagnostic_event("url_requested", serde_json::json!({ "url": url }));
            if std::env::var_os("CODEX_STATUS_DIAGNOSTIC_SUPPRESS_SHELL").is_some() {
                return;
            }
        }
        let url = wide0(url);
        let result = unsafe {
            ShellExecuteW(
                Some(self.hwnd),
                w!("open"),
                PCWSTR(url.as_ptr()),
                PCWSTR::null(),
                PCWSTR::null(),
                SW_SHOWNORMAL,
            )
        };
        if result.0 as isize <= 32 {
            self.show_balloon(
                self.locale.text("Could not open browser", "无法打开浏览器"),
                self.locale
                    .text("Copy the link from the project README.", "请从项目 README 复制链接。"),
            );
        }
    }
}

fn save_settings_change(
    store: &AppStore,
    settings: &mut Settings,
    update: impl FnOnce(&mut Settings),
) -> std::io::Result<()> {
    let previous = settings.clone();
    update(settings);
    if let Err(error) = store.save_settings(settings) {
        *settings = previous;
        return Err(error);
    }
    Ok(())
}

fn automatic_update_delay(last_check: Option<i64>, now: i64, fallback_delay_ms: u32) -> u32 {
    last_check
        .map(|last| last.saturating_add(UPDATE_INTERVAL_SECONDS).saturating_sub(now))
        .filter(|seconds| *seconds > 0)
        .and_then(|seconds| u32::try_from(seconds.saturating_mul(1_000)).ok())
        .unwrap_or(fallback_delay_ms)
        .max(1_000)
}

fn update_target_key(target: &Path) -> String {
    target.to_string_lossy().replace('/', "\\").to_ascii_lowercase()
}

fn should_show_update_target_warning(
    kind: UpdateCheckKind,
    warned_targets: &[String],
    target_key: &str,
) -> bool {
    kind == UpdateCheckKind::Manual || !warned_targets.iter().any(|path| path == target_key)
}

fn set_alert_threshold(settings: &mut Settings, kind: QuotaKind, threshold: Option<u8>) -> bool {
    let (current, last_reset) = match kind {
        QuotaKind::Weekly => (&mut settings.alert_threshold, &mut settings.last_alert_reset),
        QuotaKind::Session => {
            (&mut settings.session_alert_threshold, &mut settings.last_session_alert_reset)
        }
    };
    if *current == threshold {
        return false;
    }
    *current = threshold;
    *last_reset = None;
    true
}

#[cfg(feature = "diagnostics")]
unsafe fn diagnostic_menu_items(menu: HMENU) {
    unsafe {
        let count = GetMenuItemCount(Some(menu));
        for position in 0..count {
            let command = GetMenuItemID(menu, position);
            if command != u32::MAX && command != 0 {
                let state = GetMenuState(menu, command, MF_BYCOMMAND);
                let mut info = MENUITEMINFOW {
                    cbSize: size_of::<MENUITEMINFOW>() as u32,
                    fMask: MIIM_FTYPE,
                    ..Default::default()
                };
                let radio = GetMenuItemInfoW(menu, command, false, &mut info).is_ok()
                    && info.fType & MFT_RADIOCHECK == MFT_RADIOCHECK;
                diagnostic_event(
                    "menu_item",
                    serde_json::json!({
                        "command": command,
                        "checked": state & MF_CHECKED.0 != 0,
                        "enabled": state & (MF_DISABLED.0 | MF_GRAYED.0) == 0,
                        "radio": radio,
                    }),
                );
            }
            let submenu = GetSubMenu(menu, position);
            if !submenu.0.is_null() {
                diagnostic_menu_items(submenu);
            }
        }
    }
}

impl Drop for AppState {
    fn drop(&mut self) {
        unsafe {
            let _ = KillTimer(Some(self.hwnd), TIMER_REFRESH);
            let _ = KillTimer(Some(self.hwnd), TIMER_STARTUP);
            let _ = KillTimer(Some(self.hwnd), TIMER_CARD);
            let _ = KillTimer(Some(self.hwnd), TIMER_FLYOUT_ACTIVATE);
            let _ = KillTimer(Some(self.hwnd), TIMER_UPDATE);
            let _ = KillTimer(Some(self.hwnd), TIMER_WORKING_SET_TRIM);
            let _ = KillTimer(Some(self.hwnd), TIMER_STATUS);
            let _ = KillTimer(Some(self.hwnd), TIMER_RENDERER_RELEASE);
            let _ = KillTimer(Some(self.hwnd), TIMER_REFRESH_ANIMATION);
            let _ = KillTimer(Some(self.hwnd), TIMER_TEST_NOTIFICATION_FEEDBACK);
            let _ = KillTimer(Some(self.hwnd), TIMER_REFRESH_WATCHDOG);
            if self.refresh_timer_resolution_active {
                let _ = timeEndPeriod(1);
                self.refresh_timer_resolution_active = false;
            }
            if self.tray_added {
                let data = self.notify_data();
                let _ = Shell_NotifyIconW(NIM_DELETE, &data);
            }
        }
    }
}

unsafe fn append(
    menu: HMENU,
    command: u32,
    label: &str,
    checked: bool,
) -> windows::core::Result<()> {
    unsafe {
        let text = wide0(label);
        let flags = if checked { MF_STRING | MF_CHECKED } else { MF_STRING };
        AppendMenuW(menu, flags, command as usize, PCWSTR(text.as_ptr()))
    }
}

unsafe fn radio_group(
    menu: HMENU,
    first: u32,
    last: u32,
    selected: u32,
) -> windows::core::Result<()> {
    unsafe { CheckMenuRadioItem(menu, first, last, selected, MF_BYCOMMAND.0) }
}

unsafe fn append_command_disabled(
    menu: HMENU,
    command: u32,
    label: &str,
) -> windows::core::Result<()> {
    unsafe {
        let text = wide0(label);
        AppendMenuW(
            menu,
            MF_STRING | MF_DISABLED | MF_GRAYED,
            command as usize,
            PCWSTR(text.as_ptr()),
        )
    }
}

unsafe fn append_submenu(menu: HMENU, submenu: HMENU, label: &str) -> windows::core::Result<()> {
    unsafe {
        let text = wide0(label);
        AppendMenuW(menu, MF_POPUP, submenu.0 as usize, PCWSTR(text.as_ptr()))
    }
}

unsafe fn append_disabled(menu: HMENU, label: &str) -> windows::core::Result<()> {
    unsafe {
        let text = wide0(label);
        AppendMenuW(menu, MF_STRING | MF_DISABLED | MF_GRAYED, 0, PCWSTR(text.as_ptr()))
    }
}

unsafe fn separator(menu: HMENU) -> windows::core::Result<()> {
    unsafe { AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null()) }
}

fn cursor_rect() -> RECT {
    let mut point = POINT::default();
    unsafe {
        let _ = GetCursorPos(&mut point);
    }
    RECT { left: point.x, top: point.y, right: point.x + 1, bottom: point.y + 1 }
}

fn copy_utf16<const N: usize>(target: &mut [u16; N], value: &str) {
    target.fill(0);
    for (destination, source) in
        target.iter_mut().take(N.saturating_sub(1)).zip(value.encode_utf16())
    {
        *destination = source;
    }
}

fn mutex_name_text() -> &'static str {
    mutex_name_for(env!("CODEX_STATUS_CHANNEL"), cfg!(feature = "diagnostics"))
}

fn mutex_name_for(channel: &str, diagnostics: bool) -> &'static str {
    match (channel, diagnostics) {
        ("stable", false) => "Local\\CodexStatus.4B7D5A91-45A5-4B78-A095-A9B43A2A4F7D",
        ("stable", true) => "Local\\CodexStatus.Diagnostics.4B7D5A91-45A5-4B78-A095-A9B43A2A4F7D",
        ("beta", false) => "Local\\CodexStatus.Beta.CF8C5592-542F-47D2-A7B2-FA3EE023D0B3",
        ("beta", true) => {
            "Local\\CodexStatus.Beta.Diagnostics.CF8C5592-542F-47D2-A7B2-FA3EE023D0B3"
        }
        ("development", false) => {
            "Local\\CodexStatus.Development.C4F400E1-9A66-410C-8CD4-BABD3AAB77B1"
        }
        ("development", true) => {
            "Local\\CodexStatus.Development.Diagnostics.C4F400E1-9A66-410C-8CD4-BABD3AAB77B1"
        }
        ("portable", false) => "Local\\CodexStatus.Portable.780AC163-DB94-4E7C-8976-712402FBA7A3",
        ("portable", true) => {
            "Local\\CodexStatus.Portable.Diagnostics.780AC163-DB94-4E7C-8976-712402FBA7A3"
        }
        _ => unreachable!("build.rs validates CODEX_STATUS_CHANNEL"),
    }
}

fn wide0(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

fn seed_alert_tracker(settings: &Settings, state: &DisplayState, now: i64) -> AlertTracker {
    let mut tracker = AlertTracker::default();
    let Some(snapshot) = state.snapshot.as_ref() else {
        return tracker;
    };
    for (kind, window, last_alert_reset) in [
        (QuotaKind::Weekly, snapshot.weekly.as_ref(), settings.last_alert_reset),
        (QuotaKind::Session, snapshot.session.as_ref(), settings.last_session_alert_reset),
    ] {
        let Some(window) = window else {
            continue;
        };
        let Some(cycle) = current_cycle(window, now) else {
            continue;
        };
        let mut alert_state = tracker.for_kind(kind);
        alert_state.observed_cycle = Some(cycle);
        alert_state.depleted = window.display_percent() == 0;
        if last_alert_reset == Some(cycle.resets_at) {
            alert_state.low_alerted_cycle = Some(cycle);
        }
        tracker = tracker.with_kind(kind, alert_state);
    }
    tracker
}

fn quota_kind_label(kind: QuotaKind, locale: ui::Locale) -> &'static str {
    match kind {
        QuotaKind::Weekly => locale.text("Weekly quota", "周额度"),
        QuotaKind::Session => locale.text("5-hour quota", "5 小时额度"),
    }
}

fn quota_status_line(kind: QuotaKind, window: Option<&QuotaWindow>, locale: ui::Locale) -> String {
    let label = quota_kind_label(kind, locale);
    let Some(window) = window else {
        return format!("{label}: --");
    };
    let reset = window
        .resets_at
        .map(|value| format_local_time(value, locale))
        .unwrap_or_else(|| "--".to_owned());
    format!(
        "{label}: {}% {} · {} {reset}",
        window.display_percent(),
        locale.text("remaining", "剩余"),
        locale.text("resets", "重置")
    )
}

fn format_local_time(timestamp: i64, locale: ui::Locale) -> String {
    DateTime::<Utc>::from_timestamp(timestamp, 0)
        .map(|time| {
            let local = time.with_timezone(&Local);
            match locale {
                ui::Locale::Chinese => local.format("%m/%d %H:%M").to_string(),
                ui::Locale::English => local.format("%Y-%m-%d %H:%M").to_string(),
            }
        })
        .unwrap_or_else(|| "--".to_owned())
}

fn diagnostic_window(window: Option<&QuotaWindow>) -> String {
    window.map_or_else(
        || "none".to_owned(),
        |window| {
            format!(
                "remaining={}%,window_minutes={},resets_at={}",
                window.display_percent(),
                window.window_minutes,
                window
                    .resets_at
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "none".to_owned())
            )
        },
    )
}

fn friendly_error(error: &str, locale: ui::Locale) -> String {
    let lower = error.to_ascii_lowercase();
    if lower.contains("not installed") || lower.contains("not available on path") {
        return locale
            .text("Codex is not installed or not on PATH", "未找到 Codex，请先安装并加入 PATH")
            .to_owned();
    }
    if lower.contains("not logged") || lower.contains("login") || lower.contains("unauthorized") {
        return locale
            .text("Sign in to Codex, then refresh", "请先登录 Codex，然后刷新")
            .to_owned();
    }
    if lower.contains("within 8 seconds") || lower.contains("timed out") {
        return locale.text("Codex did not respond in time", "Codex 响应超时").to_owned();
    }
    error.chars().take(180).collect()
}

#[cfg(feature = "diagnostics")]
fn diagnostic(stage: &str) {
    diagnostic_event("stage", serde_json::json!({ "name": stage }));
}

#[cfg(not(feature = "diagnostics"))]
fn diagnostic(_stage: &str) {}

#[cfg(feature = "diagnostics")]
fn diagnostic_event(event: &str, details: serde_json::Value) {
    use std::io::Write;

    let record = serde_json::json!({
        "timestamp": Utc::now().to_rfc3339(),
        "event": event,
        "details": details,
    });
    eprintln!("{record}");
    let path = std::env::var_os("CODEX_STATUS_DIAGNOSTIC_LOG")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("CodexStatus-diagnostic.jsonl"));
    if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(file, "{record}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{AccountSummary, WEEK_MINUTES};

    #[test]
    fn rejects_unknown_options() {
        assert_eq!(
            friendly_error("Codex is not installed", ui::Locale::Chinese),
            "未找到 Codex，请先安装并加入 PATH"
        );
    }

    #[test]
    fn utf16_copy_always_reserves_a_terminator() {
        let mut target = [9_u16; 4];
        copy_utf16(&mut target, "abcdef");
        assert_eq!(target[3], 0);
    }

    #[test]
    fn mutex_names_isolate_every_channel_and_diagnostics_build() {
        let mut names = std::collections::HashSet::new();
        for channel in ["stable", "beta", "development", "portable"] {
            let normal = mutex_name_for(channel, false);
            let diagnostics = mutex_name_for(channel, true);
            assert_ne!(normal, diagnostics);
            assert!(names.insert(normal));
            assert!(names.insert(diagnostics));
        }
        assert_eq!(names.len(), 8);
        assert_eq!(
            mutex_name_for("stable", false),
            "Local\\CodexStatus.4B7D5A91-45A5-4B78-A095-A9B43A2A4F7D"
        );
        assert_eq!(
            mutex_name_for("stable", true),
            "Local\\CodexStatus.Diagnostics.4B7D5A91-45A5-4B78-A095-A9B43A2A4F7D"
        );
        assert_eq!(
            mutex_name_text(),
            mutex_name_for(env!("CODEX_STATUS_CHANNEL"), cfg!(feature = "diagnostics"))
        );
    }

    #[test]
    fn persisted_cycle_prevents_a_duplicate_weekly_alert_after_restart() {
        let now = 1_000_000;
        let reset = now + 60;
        let display = DisplayState::loading(Some(QuotaSnapshot {
            weekly: Some(QuotaWindow {
                used_percent: 92.0,
                remaining_percent: 8.0,
                window_minutes: WEEK_MINUTES,
                resets_at: Some(reset),
            }),
            session: None,
            account: AccountSummary::default(),
            fetched_at: now,
        }));
        let settings = Settings {
            alert_threshold: Some(10),
            last_alert_reset: Some(reset),
            ..Settings::default()
        };
        let tracker = seed_alert_tracker(&settings, &display, now);
        let decision = evaluate_alerts(
            tracker,
            QuotaKind::Weekly,
            display.snapshot.as_ref().and_then(|value| value.weekly.as_ref()),
            settings.alert_threshold,
            now,
        );
        assert!(!decision.should_notify_low);
    }

    #[test]
    fn copied_status_omits_an_unavailable_optional_window() {
        let line = quota_status_line(QuotaKind::Session, None, ui::Locale::Chinese);
        assert_eq!(line, "5 小时额度: --");
        assert_eq!(format_local_time(i64::MAX, ui::Locale::English), "--");
    }

    #[test]
    fn failed_settings_change_rolls_back_memory_state() {
        use std::fs;
        use std::time::{SystemTime, UNIX_EPOCH};

        let suffix = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let blocker = std::env::temp_dir().join(format!("codex-status-blocker-{suffix}"));
        fs::write(&blocker, b"not a directory").unwrap();
        let store = AppStore::at(blocker.join("settings-root"));
        let mut settings = Settings::default();

        let result = save_settings_change(&store, &mut settings, |value| value.refresh_minutes = 1);

        assert!(result.is_err());
        assert_eq!(settings, Settings::default());
        fs::remove_file(blocker).unwrap();
    }

    #[test]
    fn reselecting_an_alert_threshold_keeps_duplicate_suppression() {
        let mut settings = Settings {
            alert_threshold: Some(30),
            last_alert_reset: Some(123),
            ..Settings::default()
        };

        assert!(!set_alert_threshold(&mut settings, QuotaKind::Weekly, Some(30)));
        assert_eq!(settings.last_alert_reset, Some(123));

        assert!(set_alert_threshold(&mut settings, QuotaKind::Weekly, Some(20)));
        assert_eq!(settings.alert_threshold, Some(20));
        assert_eq!(settings.last_alert_reset, None);
    }

    #[test]
    fn manual_update_checks_bypass_throttle_without_recording_daily_state() {
        assert!(UpdateCheckKind::Manual.bypasses_daily_throttle());
        assert!(!UpdateCheckKind::Manual.records_last_check());
        assert!(!UpdateCheckKind::Automatic.bypasses_daily_throttle());
        assert!(UpdateCheckKind::Automatic.records_last_check());
    }

    #[test]
    fn automatic_target_warnings_are_per_path_but_manual_checks_always_report() {
        let target = update_target_key(Path::new(r"C:\Tools\quota.exe"));
        assert!(should_show_update_target_warning(UpdateCheckKind::Automatic, &[], &target));
        assert!(!should_show_update_target_warning(
            UpdateCheckKind::Automatic,
            std::slice::from_ref(&target),
            &target
        ));
        assert!(should_show_update_target_warning(
            UpdateCheckKind::Automatic,
            std::slice::from_ref(&target),
            &update_target_key(Path::new(r"C:\Other\quota.exe"))
        ));
        assert!(should_show_update_target_warning(
            UpdateCheckKind::Manual,
            std::slice::from_ref(&target),
            &target
        ));
    }

    #[test]
    fn update_target_warning_keys_follow_windows_path_identity() {
        assert_eq!(
            update_target_key(Path::new(r"C:/Tools/CodexStatus.exe")),
            update_target_key(Path::new(r"c:\tools\codexstatus.exe"))
        );
    }

    #[test]
    fn action_required_and_test_notifications_bypass_quiet_time() {
        assert!(NotificationKind::Alert.respects_quiet_time());
        assert!(!NotificationKind::ActionRequired.respects_quiet_time());
        assert!(!NotificationKind::Test.respects_quiet_time());
    }

    #[test]
    fn automatic_update_delay_still_uses_the_daily_timestamp() {
        let now = 1_000_000;
        assert_eq!(
            automatic_update_delay(Some(now - 60), now, UPDATE_INITIAL_DELAY_MS),
            (UPDATE_INTERVAL_SECONDS as u32 - 60) * 1_000
        );
        assert_eq!(
            automatic_update_delay(
                Some(now - UPDATE_INTERVAL_SECONDS),
                now,
                UPDATE_INITIAL_DELAY_MS
            ),
            UPDATE_INITIAL_DELAY_MS
        );
    }

    #[test]
    fn forced_refreshes_coalesce_and_stale_completions_cannot_finish_a_new_cycle() {
        let mut sequence = RefreshSequence::default();
        let first = sequence.begin(true, false).unwrap();
        assert!(sequence.begin(true, false).is_none());
        assert_eq!(sequence.finish(first), Some(true));

        let second = sequence.begin(true, false).unwrap();
        assert_ne!(first, second);
        assert_eq!(sequence.finish(first), None);
        assert_eq!(sequence.active_id(), Some(second));
        assert_eq!(sequence.finish(second), Some(false));
    }

    #[test]
    fn automatic_refresh_does_not_queue_behind_an_active_cycle() {
        let mut sequence = RefreshSequence::default();
        let active = sequence.begin(false, false).unwrap();
        assert!(sequence.begin(false, false).is_none());
        assert_eq!(sequence.finish(active), Some(false));
    }

    #[test]
    fn paused_refresh_requests_do_not_start_until_resume() {
        let mut sequence = RefreshSequence::default();
        assert!(sequence.begin(true, true).is_none());
        sequence.clear_pending();
        assert!(sequence.begin(false, false).is_some());
    }
}
