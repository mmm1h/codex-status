use crate::insights::analyze_window;
use crate::model::{DisplayState, QuotaWindow, RefreshState};
use chrono::{DateTime, Local};
use std::ffi::c_void;
use std::mem::size_of;
use windows::Win32::Foundation::{COLORREF, HWND, RECT};
use windows::Win32::Globalization::GetUserDefaultLocaleName;
use windows::Win32::Graphics::Dwm::{
    DWMWA_USE_IMMERSIVE_DARK_MODE, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND,
    DwmSetWindowAttribute,
};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, BitBlt, CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, CreateCompatibleBitmap,
    CreateCompatibleDC, CreateFontW, CreateRoundRectRgn, CreateSolidBrush, DEFAULT_CHARSET,
    DEFAULT_PITCH, DT_END_ELLIPSIS, DT_LEFT, DT_NOPREFIX, DT_SINGLELINE, DT_VCENTER, DeleteDC,
    DeleteObject, DrawTextW, EndPaint, FF_SWISS, FONT_QUALITY, FW_NORMAL, FW_SEMIBOLD, FillRect,
    FillRgn, GetTextExtentPoint32W, HDC, HGDIOBJ, InvalidateRect, OUT_DEFAULT_PRECIS, PAINTSTRUCT,
    SRCCOPY, SelectObject, SetBkMode, SetTextColor, TRANSPARENT,
};
use windows::Win32::UI::Accessibility::{HCF_HIGHCONTRASTON, HIGHCONTRASTW};
use windows::Win32::UI::WindowsAndMessaging::{
    GetClientRect, MB_ICONERROR, MB_OK, MESSAGEBOX_STYLE, MessageBoxW, SPI_GETHIGHCONTRAST,
    SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, SystemParametersInfoW,
};
use windows::core::{PCWSTR, w};
use winreg::RegKey;
use winreg::enums::HKEY_CURRENT_USER;

mod backdrop;
mod direct2d;

pub const CARD_WIDTH: i32 = 420;
pub const CARD_HEIGHT: i32 = 430;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum RefreshButtonState {
    #[default]
    Idle,
    Hovered,
    Pressed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Locale {
    English,
    Chinese,
}

impl Locale {
    pub fn detect(setting: &str) -> Self {
        match setting {
            "en" => Self::English,
            "zh-CN" => Self::Chinese,
            _ => {
                let mut name = [0_u16; 85];
                let length = unsafe { GetUserDefaultLocaleName(&mut name) };
                let locale = if length > 0 {
                    String::from_utf16_lossy(&name[..length.saturating_sub(1) as usize])
                } else {
                    String::new()
                };
                if locale.to_ascii_lowercase().starts_with("zh") {
                    Self::Chinese
                } else {
                    Self::English
                }
            }
        }
    }

    pub fn text(self, english: &'static str, chinese: &'static str) -> &'static str {
        match self {
            Self::English => english,
            Self::Chinese => chinese,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Theme {
    pub dark: bool,
    pub tray_dark: bool,
    pub high_contrast: bool,
    background: COLORREF,
    surface: COLORREF,
    surface_alt: COLORREF,
    text: COLORREF,
    muted: COLORREF,
    line: COLORREF,
}

pub fn detect_theme(preference: &str) -> Theme {
    let mut high_contrast =
        HIGHCONTRASTW { cbSize: size_of::<HIGHCONTRASTW>() as u32, ..Default::default() };
    let high_contrast_enabled = unsafe {
        SystemParametersInfoW(
            SPI_GETHIGHCONTRAST,
            high_contrast.cbSize,
            Some((&mut high_contrast as *mut HIGHCONTRASTW).cast::<c_void>()),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
        .is_ok()
            && high_contrast.dwFlags.contains(HCF_HIGHCONTRASTON)
    };
    let personalize = RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize")
        .ok();
    let system_dark = personalize
        .as_ref()
        .and_then(|key| key.get_value::<u32, _>("AppsUseLightTheme").ok())
        .is_some_and(|value| value == 0);
    let dark = match preference {
        "light" => false,
        "dark" => true,
        _ => system_dark,
    };
    let tray_dark = personalize
        .as_ref()
        .and_then(|key| key.get_value::<u32, _>("SystemUsesLightTheme").ok())
        .is_some_and(|value| value == 0);

    if high_contrast_enabled {
        Theme {
            dark: true,
            tray_dark: true,
            high_contrast: true,
            background: rgb(0, 0, 0),
            surface: rgb(0, 0, 0),
            surface_alt: rgb(0, 0, 0),
            text: rgb(255, 255, 255),
            muted: rgb(255, 255, 255),
            line: rgb(255, 255, 255),
        }
    } else if dark {
        Theme {
            dark,
            tray_dark,
            high_contrast: false,
            background: rgb(31, 34, 37),
            surface: rgb(45, 48, 51),
            surface_alt: rgb(36, 39, 42),
            text: rgb(240, 243, 242),
            muted: rgb(154, 161, 158),
            line: rgb(72, 78, 76),
        }
    } else {
        Theme {
            dark,
            tray_dark,
            high_contrast: false,
            background: rgb(229, 233, 239),
            surface: rgb(247, 249, 251),
            surface_alt: rgb(222, 228, 234),
            text: rgb(32, 37, 43),
            muted: rgb(102, 112, 122),
            line: rgb(198, 207, 216),
        }
    }
}

pub fn configure_flyout(hwnd: HWND, theme: Theme) {
    unsafe {
        let dark = i32::from(theme.dark);
        let corner = DWMWCP_ROUND;
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_USE_IMMERSIVE_DARK_MODE,
            (&dark as *const i32).cast(),
            size_of::<i32>() as u32,
        );
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            (&corner as *const windows::Win32::Graphics::Dwm::DWM_WINDOW_CORNER_PREFERENCE).cast(),
            size_of_val(&corner) as u32,
        );
    }
    backdrop::configure(hwnd, theme);
}

pub fn show_fatal_error(message: &str) {
    let body = wide0(message);
    unsafe {
        let _ = MessageBoxW(
            None,
            PCWSTR(body.as_ptr()),
            w!("CodexStatus"),
            MESSAGEBOX_STYLE(MB_OK.0 | MB_ICONERROR.0),
        );
    }
}

pub fn tray_percent(state: &DisplayState, metric: &str) -> Option<u8> {
    let weekly = state.weekly_percent();
    let session = state.session_percent();
    match metric {
        "session" => session.or(weekly),
        "lowest" => match (weekly, session) {
            (Some(weekly), Some(session)) => Some(weekly.min(session)),
            (weekly, session) => weekly.or(session),
        },
        _ => weekly.or(session),
    }
}

pub fn tooltip_for_metric(state: &DisplayState, locale: Locale, metric: &str) -> String {
    let status = match state.refresh_state {
        RefreshState::Loading => locale.text("refreshing", "刷新中"),
        RefreshState::Live => locale.text("live", "实时"),
        RefreshState::Cached => locale.text("cached", "缓存"),
        RefreshState::Unavailable => locale.text("unavailable", "不可用"),
    };
    let label = match metric {
        "session" if state.session_percent().is_some() => {
            locale.text("5-hour remaining", "5 小时剩余")
        }
        "lowest" if state.weekly_percent().is_some() && state.session_percent().is_some() => {
            locale.text("lowest remaining", "较低额度剩余")
        }
        _ => locale.text("weekly remaining", "周剩余"),
    };
    match tray_percent(state, metric) {
        Some(percent) => format!("CodexStatus · {label} {percent}% · {status}",),
        None => format!("CodexStatus · {status}"),
    }
}

pub fn paint_card(
    hwnd: HWND,
    state: &DisplayState,
    locale: Locale,
    theme: Theme,
    refresh_button: RefreshButtonState,
    refreshing: bool,
) {
    unsafe {
        let mut paint = PAINTSTRUCT::default();
        let hdc = BeginPaint(hwnd, &mut paint);
        let mut client = RECT::default();
        let _ = GetClientRect(hwnd, &mut client);
        let width = (client.right - client.left).max(1);
        let height = (client.bottom - client.top).max(1);
        let dpi = windows::Win32::UI::HiDpi::GetDpiForWindow(hwnd).max(96);

        if !direct2d::paint(direct2d::PaintInput {
            hwnd,
            size: (width, height),
            dpi,
            state,
            locale,
            theme,
            refresh_button,
            refreshing,
            glass_enabled: backdrop::glass_enabled(hwnd),
        }) {
            let requires_opaque = direct2d::take_gdi_frame_requires_opaque();
            if direct2d::gdi_fallback_active() || requires_opaque {
                // GDI cannot provide premultiplied composition content. Remove
                // acrylic so the opaque fallback owns the first visible frame.
                backdrop::disable_for_render_fallback(hwnd);
            }
            let buffer = CreateCompatibleDC(Some(hdc));
            let bitmap = CreateCompatibleBitmap(hdc, width, height);
            if !buffer.is_invalid() && !bitmap.is_invalid() {
                let old_bitmap = SelectObject(buffer, HGDIOBJ(bitmap.0));
                draw_card(buffer, state, locale, theme, refresh_button, refreshing, dpi);
                let _ = BitBlt(hdc, 0, 0, width, height, Some(buffer), 0, 0, SRCCOPY);
                let _ = SelectObject(buffer, old_bitmap);
            } else {
                draw_card(hdc, state, locale, theme, refresh_button, refreshing, dpi);
            }
            if !bitmap.is_invalid() {
                let _ = DeleteObject(HGDIOBJ(bitmap.0));
            }
            if !buffer.is_invalid() {
                let _ = DeleteDC(buffer);
            }
        }
        let followup_paint = direct2d::take_followup_paint_request();
        let _ = EndPaint(hwnd, &paint);
        if followup_paint {
            let _ = InvalidateRect(Some(hwnd), None, false);
        }
    }
}

pub fn release_card_surface() {
    direct2d::release_surface();
}

pub fn release_card_device_tree() {
    direct2d::release_device_tree();
}

#[allow(clippy::too_many_arguments)]
unsafe fn draw_card(
    hdc: HDC,
    state: &DisplayState,
    locale: Locale,
    theme: Theme,
    refresh_button: RefreshButtonState,
    refreshing: bool,
    dpi: u32,
) {
    unsafe {
        let width = scale(CARD_WIDTH, dpi);
        let height = scale(CARD_HEIGHT, dpi);
        fill(hdc, RECT { left: 0, top: 0, right: width, bottom: height }, theme.background);
        let _ = SetBkMode(hdc, TRANSPARENT);

        let status_color = accent_for(state, theme);
        fill_rounded(
            hdc,
            RECT {
                left: scale(20, dpi),
                top: scale(26, dpi),
                right: scale(28, dpi),
                bottom: scale(34, dpi),
            },
            scale(8, dpi),
            status_color,
        );
        draw_text(
            hdc,
            locale,
            "Codex",
            RECT {
                left: scale(39, dpi),
                top: scale(9, dpi),
                right: scale(102, dpi),
                bottom: scale(52, dpi),
            },
            scale(14, dpi),
            FW_SEMIBOLD.0 as i32,
            theme.text,
        );

        draw_text(
            hdc,
            locale,
            &updated_text(state),
            RECT {
                left: scale(108, dpi),
                top: scale(10, dpi),
                right: scale(245, dpi),
                bottom: scale(51, dpi),
            },
            scale(12, dpi),
            FW_NORMAL.0 as i32,
            theme.muted,
        );
        draw_refresh_button(hdc, theme, refresh_button, refreshing, dpi);

        let hero = RECT {
            left: scale(16, dpi),
            top: scale(64, dpi),
            right: width - scale(16, dpi),
            bottom: scale(385, dpi),
        };
        if theme.high_contrast {
            outlined_surface(hdc, hero, scale(8, dpi), theme.surface, theme.line, dpi);
        } else {
            fill_rounded(hdc, hero, scale(8, dpi), theme.surface);
        }

        draw_percentage(hdc, state.weekly_percent(), locale, theme, status_color, dpi);
        draw_text(
            hdc,
            locale,
            locale.text("Weekly remaining", "本周剩余"),
            RECT {
                left: scale(32, dpi),
                top: scale(145, dpi),
                right: scale(230, dpi),
                bottom: scale(176, dpi),
            },
            scale(14, dpi),
            FW_NORMAL.0 as i32,
            theme.muted,
        );

        let reset = state
            .snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.weekly.as_ref())
            .map(|window| reset_details(window, locale))
            .unwrap_or_else(|| {
                (
                    locale.text("Unavailable", "暂无").to_owned(),
                    locale.text("Reset time", "重置时间").to_owned(),
                )
            });
        draw_text(
            hdc,
            locale,
            locale.text("Reset in", "距离重置"),
            RECT {
                left: scale(32, dpi),
                top: scale(235, dpi),
                right: width - scale(34, dpi),
                bottom: scale(264, dpi),
            },
            scale(12, dpi),
            FW_NORMAL.0 as i32,
            theme.muted,
        );
        draw_text(
            hdc,
            locale,
            &reset.0,
            RECT {
                left: scale(32, dpi),
                top: scale(258, dpi),
                right: width - scale(34, dpi),
                bottom: scale(291, dpi),
            },
            scale(20, dpi),
            FW_SEMIBOLD.0 as i32,
            theme.text,
        );
        draw_text(
            hdc,
            locale,
            &reset.1,
            RECT {
                left: scale(32, dpi),
                top: scale(286, dpi),
                right: width - scale(34, dpi),
                bottom: scale(312, dpi),
            },
            scale(12, dpi),
            FW_NORMAL.0 as i32,
            theme.muted,
        );

        let bar = RECT {
            left: scale(32, dpi),
            top: scale(181, dpi),
            right: width - scale(32, dpi),
            bottom: scale(189, dpi),
        };
        fill_rounded(hdc, bar, scale(4, dpi), theme.line);
        if let Some(value) = state.weekly_percent() {
            let filled = (bar.left + (bar.right - bar.left) * i32::from(value) / 100)
                .max(bar.left + (bar.bottom - bar.top));
            if value > 0 {
                fill_rounded(
                    hdc,
                    RECT { right: filled.min(bar.right), ..bar },
                    scale(4, dpi),
                    status_color,
                );
            }
        }
        let now = Local::now().timestamp();
        let pace_insight = state
            .snapshot
            .as_ref()
            .and_then(|snapshot| {
                snapshot.weekly.as_ref().map(|window| analyze_window(window, snapshot.fetched_at))
            })
            .filter(|insight| insight.reset_at.is_some_and(|reset_at| reset_at > now));
        let projection = weekly_usage_projection(state, now);
        let pace_text = projection.map_or_else(
            || {
                if pace_insight.is_some_and(|insight| insight.elapsed_percent.is_some()) {
                    locale.text("Usage pace on track", "用量节奏正常").to_owned()
                } else {
                    locale.text("Waiting for usage pace", "等待用量节奏数据").to_owned()
                }
            },
            |value| projection_label(value, locale).text,
        );
        draw_text(
            hdc,
            locale,
            &pace_text,
            RECT {
                left: scale(32, dpi),
                top: scale(192, dpi),
                right: width - scale(32, dpi),
                bottom: scale(225, dpi),
            },
            scale(14, dpi),
            FW_NORMAL.0 as i32,
            theme.muted,
        );

        let session = state
            .snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.session.as_ref())
            .map(|window| format!("{}%", window.display_percent()));
        let plan = state
            .snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.account.plan_type.as_deref())
            .map(|plan| plan_label(plan, locale))
            .unwrap_or("--")
            .to_owned();
        let credits = state
            .snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.account.reset_credits)
            .map(|credits| format!("{credits} {}", locale.text("resets", "次")))
            .unwrap_or_else(|| "--".to_owned());

        let metrics = RECT {
            left: scale(16, dpi),
            top: scale(315, dpi),
            right: width - scale(16, dpi),
            bottom: scale(385, dpi),
        };
        if theme.high_contrast {
            outlined_surface(hdc, metrics, scale(8, dpi), theme.surface_alt, theme.line, dpi);
        } else {
            fill_rounded(hdc, metrics, scale(8, dpi), theme.surface_alt);
        }
        if let Some(session) = session {
            metric_column(
                hdc,
                locale,
                RECT { right: scale(151, dpi), ..metrics },
                locale.text("Plan", "套餐"),
                &plan,
                theme,
                dpi,
            );
            metric_column(
                hdc,
                locale,
                RECT { left: scale(151, dpi), right: scale(282, dpi), ..metrics },
                locale.text("Session quota", "会话额度"),
                &session,
                theme,
                dpi,
            );
            metric_column(
                hdc,
                locale,
                RECT { left: scale(282, dpi), ..metrics },
                locale.text("Reset credits", "重置机会"),
                &credits,
                theme,
                dpi,
            );
        } else {
            metric_column(
                hdc,
                locale,
                RECT { right: scale(220, dpi), ..metrics },
                locale.text("Plan", "套餐"),
                &plan,
                theme,
                dpi,
            );
            metric_column(
                hdc,
                locale,
                RECT { left: scale(220, dpi), ..metrics },
                locale.text("Reset credits", "重置机会"),
                &credits,
                theme,
                dpi,
            );
        }

        let footer = footer_text(state, locale);
        draw_text(
            hdc,
            locale,
            &footer,
            RECT {
                left: scale(32, dpi),
                top: scale(390, dpi),
                right: width - scale(18, dpi),
                bottom: height,
            },
            scale(12, dpi),
            FW_NORMAL.0 as i32,
            if state.error.is_some() { accent_red(theme) } else { theme.muted },
        );
    }
}

pub fn refresh_hit_test(x: i32, y: i32, dpi: u32) -> bool {
    let rect = refresh_rect(dpi);
    x >= rect.left && x < rect.right && y >= rect.top && y < rect.bottom
}

pub fn refresh_rect(dpi: u32) -> RECT {
    RECT {
        left: scale(362, dpi),
        top: scale(8, dpi),
        right: scale(416, dpi),
        bottom: scale(60, dpi),
    }
}

unsafe fn draw_refresh_button(
    hdc: HDC,
    theme: Theme,
    state: RefreshButtonState,
    refreshing: bool,
    dpi: u32,
) {
    unsafe {
        let rect = RECT {
            left: scale(371, dpi),
            top: scale(16, dpi),
            right: scale(407, dpi),
            bottom: scale(52, dpi),
        };
        let fill_color = refresh_button_fill(theme, state);
        if theme.high_contrast {
            outlined_surface(hdc, rect, scale(4, dpi), fill_color, theme.line, dpi);
        } else {
            fill_rounded(hdc, rect, scale(4, dpi), fill_color);
        }
        draw_text(
            hdc,
            Locale::English,
            if refreshing { "…" } else { "↻" },
            RECT {
                left: rect.left + scale(if refreshing { 9 } else { 7 }, dpi),
                top: rect.top,
                right: rect.right,
                bottom: rect.bottom,
            },
            scale(if refreshing { 18 } else { 21 }, dpi),
            FW_NORMAL.0 as i32,
            if state == RefreshButtonState::Pressed { theme.text } else { theme.muted },
        );
    }
}

unsafe fn draw_percentage(
    hdc: HDC,
    percent: Option<u8>,
    locale: Locale,
    theme: Theme,
    accent: COLORREF,
    dpi: u32,
) {
    unsafe {
        let Some(percent) = percent else {
            draw_text(
                hdc,
                locale,
                "--",
                RECT {
                    left: scale(32, dpi),
                    top: scale(68, dpi),
                    right: scale(250, dpi),
                    bottom: scale(151, dpi),
                },
                scale(68, dpi),
                FW_SEMIBOLD.0 as i32,
                theme.text,
            );
            return;
        };
        let number = percent.to_string();
        draw_text(
            hdc,
            locale,
            &number,
            RECT {
                left: scale(32, dpi),
                top: scale(68, dpi),
                right: scale(250, dpi),
                bottom: scale(151, dpi),
            },
            scale(68, dpi),
            FW_SEMIBOLD.0 as i32,
            accent,
        );
        let number_left = scale(32, dpi);
        let number_width =
            measure_text_width(hdc, locale, &number, scale(68, dpi), FW_SEMIBOLD.0 as i32);
        draw_text(
            hdc,
            locale,
            "%",
            RECT {
                left: number_left + number_width + scale(3, dpi),
                top: scale(104, dpi),
                right: scale(270, dpi),
                bottom: scale(150, dpi),
            },
            scale(20, dpi),
            FW_NORMAL.0 as i32,
            accent,
        );
    }
}

unsafe fn metric_column(
    hdc: HDC,
    locale: Locale,
    rect: RECT,
    label: &str,
    value: &str,
    theme: Theme,
    dpi: u32,
) {
    unsafe {
        draw_text(
            hdc,
            locale,
            label,
            RECT {
                left: rect.left + scale(16, dpi),
                top: rect.top + scale(5, dpi),
                right: rect.right - scale(8, dpi),
                bottom: rect.top + scale(34, dpi),
            },
            scale(12, dpi),
            FW_NORMAL.0 as i32,
            theme.muted,
        );
        draw_text(
            hdc,
            locale,
            value,
            RECT {
                left: rect.left + scale(16, dpi),
                top: rect.top + scale(30, dpi),
                right: rect.right - scale(8, dpi),
                bottom: rect.bottom - scale(4, dpi),
            },
            scale(20, dpi),
            FW_SEMIBOLD.0 as i32,
            theme.text,
        );
    }
}

unsafe fn outlined_surface(
    hdc: HDC,
    rect: RECT,
    radius: i32,
    surface: COLORREF,
    border_color: COLORREF,
    dpi: u32,
) {
    unsafe {
        fill_rounded(hdc, rect, radius, border_color);
        let border = scale(1, dpi).max(1);
        fill_rounded(
            hdc,
            RECT {
                left: rect.left + border,
                top: rect.top + border,
                right: rect.right - border,
                bottom: rect.bottom - border,
            },
            (radius - border).max(1),
            surface,
        );
    }
}

fn updated_text(state: &DisplayState) -> String {
    state
        .snapshot
        .as_ref()
        .and_then(|snapshot| DateTime::from_timestamp(snapshot.fetched_at, 0))
        .map(|time| time.with_timezone(&Local).format("%H:%M").to_string())
        .unwrap_or_else(|| "--:--".to_owned())
}

fn footer_text(state: &DisplayState, locale: Locale) -> String {
    if let Some(error) = state.error.as_deref() {
        let prefix = if state.weekly_percent().is_some() {
            locale.text("Cached · ", "缓存 · ")
        } else {
            locale.text("Unavailable · ", "不可用 · ")
        };
        return format!("{prefix}{error}");
    }
    if state.refresh_state == RefreshState::Loading {
        return locale.text("Refreshing Codex quota…", "正在刷新 Codex 额度…").to_owned();
    }
    if state.snapshot.is_some() {
        locale.text("Read only from local Codex", "仅从本机 Codex 读取").to_owned()
    } else {
        locale.text("Waiting for Codex", "等待 Codex 数据").to_owned()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UsageProjection {
    Exhausted,
    DepletesIn { seconds: i64 },
}

fn weekly_usage_projection(state: &DisplayState, now: i64) -> Option<UsageProjection> {
    let snapshot = state.snapshot.as_ref()?;
    let window = snapshot.weekly.as_ref()?;
    let insight = analyze_window(window, snapshot.fetched_at);
    let used_percent = insight.used_percent?;
    if used_percent >= 100.0 {
        return Some(UsageProjection::Exhausted);
    }

    let reset_at = insight.reset_at?;
    if reset_at <= now || !insight.is_ahead_of_pace || !insight.likely_exhaust_before_reset {
        return None;
    }
    let projected_at = insight.projected_exhaustion_at?;
    Some(UsageProjection::DepletesIn { seconds: projected_at.saturating_sub(now).max(0) })
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ProjectionLabel {
    text: String,
}

fn projection_label(projection: UsageProjection, locale: Locale) -> ProjectionLabel {
    let text = match projection {
        UsageProjection::Exhausted => locale.text("Quota exhausted", "额度耗尽").to_owned(),
        UsageProjection::DepletesIn { seconds } => {
            let total_hours = if seconds <= 0 { 0 } else { seconds.saturating_add(3_599) / 3_600 };
            let days = total_hours / 24;
            let hours = total_hours % 24;
            if locale == Locale::Chinese {
                if days > 0 {
                    format!("用量偏快 · 约{days}天{hours}小时后耗尽")
                } else {
                    format!("用量偏快 · 约{hours}小时后耗尽")
                }
            } else if days > 0 {
                format!("Pace high · empty in ~{days}d {hours}h")
            } else {
                format!("Pace high · empty in ~{hours}h")
            }
        }
    };
    ProjectionLabel { text }
}

fn reset_details(window: &QuotaWindow, locale: Locale) -> (String, String) {
    let Some(reset) = window.resets_at else {
        return (
            locale.text("Unavailable", "暂无").to_owned(),
            locale.text("Reset time", "重置时间").to_owned(),
        );
    };
    let now = Local::now().timestamp();
    let seconds = reset.saturating_sub(now).max(0);
    let days = seconds / 86_400;
    let hours = (seconds % 86_400) / 3_600;
    let minutes = (seconds % 3_600) / 60;
    let countdown = if locale == Locale::Chinese {
        if days > 0 {
            format!("{days} 天 {hours} 小时")
        } else {
            format!("{hours} 小时 {minutes} 分")
        }
    } else if days > 0 {
        format!("{days}d {hours}h")
    } else {
        format!("{hours}h {minutes}m")
    };
    let local_time = DateTime::from_timestamp(reset, 0)
        .map(|time| time.with_timezone(&Local).format("%m/%d %H:%M").to_string())
        .unwrap_or_else(|| "--".to_owned());
    (countdown, local_time)
}

pub(crate) fn plan_label(plan: &str, locale: Locale) -> &str {
    match plan.to_ascii_lowercase().as_str() {
        "free" => locale.text("Free", "免费"),
        "go" => "Go",
        "plus" => "Plus",
        "prolite" => "Pro Lite",
        "pro" => "Pro",
        "team" | "self_serve_business_usage_based" => "Business",
        "business" | "ent26" | "enterprise_cbp_usage_based" => "Enterprise",
        "edu" => locale.text("Edu", "教育版"),
        _ => plan,
    }
}

fn accent_for(state: &DisplayState, theme: Theme) -> COLORREF {
    if theme.high_contrast {
        return theme.text;
    }
    match state.weekly_percent() {
        Some(value) if value < 20 => accent_red(theme),
        Some(value) if value < 50 => accent_amber(theme),
        Some(_) => accent_green(theme),
        None => {
            if theme.dark {
                rgb(123, 139, 136)
            } else {
                rgb(105, 121, 126)
            }
        }
    }
}

const fn accent_green(theme: Theme) -> COLORREF {
    if theme.dark { rgb(27, 170, 135) } else { rgb(16, 163, 127) }
}

const fn accent_amber(theme: Theme) -> COLORREF {
    if theme.dark { rgb(197, 138, 50) } else { rgb(169, 107, 22) }
}

const fn accent_red(theme: Theme) -> COLORREF {
    if theme.dark { rgb(208, 106, 115) } else { rgb(185, 75, 85) }
}

const fn refresh_button_fill(theme: Theme, state: RefreshButtonState) -> COLORREF {
    if theme.high_contrast {
        return theme.background;
    }
    match (theme.dark, state) {
        (false, RefreshButtonState::Idle) => rgb(221, 226, 231),
        (false, RefreshButtonState::Hovered) => rgb(207, 214, 220),
        (false, RefreshButtonState::Pressed) => rgb(190, 199, 207),
        (true, RefreshButtonState::Idle) => rgb(55, 58, 61),
        (true, RefreshButtonState::Hovered) => rgb(67, 71, 74),
        (true, RefreshButtonState::Pressed) => rgb(31, 34, 37),
    }
}

unsafe fn draw_text(
    hdc: HDC,
    locale: Locale,
    value: &str,
    rect: RECT,
    height: i32,
    weight: i32,
    color: COLORREF,
) {
    let style = TextStyle { height, weight, color };
    unsafe { draw_text_with_alignment(hdc, locale, value, rect, style, DT_LEFT) }
}

#[derive(Clone, Copy)]
struct TextStyle {
    height: i32,
    weight: i32,
    color: COLORREF,
}

unsafe fn draw_text_with_alignment(
    hdc: HDC,
    locale: Locale,
    value: &str,
    mut rect: RECT,
    style: TextStyle,
    alignment: windows::Win32::Graphics::Gdi::DRAW_TEXT_FORMAT,
) {
    unsafe {
        let font = create_ui_font(locale, style.height, style.weight);
        let old = SelectObject(hdc, HGDIOBJ(font.0));
        let _ = SetTextColor(hdc, style.color);
        let mut text: Vec<u16> = value.encode_utf16().collect();
        let _ = DrawTextW(
            hdc,
            &mut text,
            &mut rect,
            alignment | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX | DT_END_ELLIPSIS,
        );
        let _ = SelectObject(hdc, old);
        let _ = DeleteObject(HGDIOBJ(font.0));
    }
}

unsafe fn measure_text_width(
    hdc: HDC,
    locale: Locale,
    value: &str,
    height: i32,
    weight: i32,
) -> i32 {
    unsafe {
        let font = create_ui_font(locale, height, weight);
        let old = SelectObject(hdc, HGDIOBJ(font.0));
        let text: Vec<u16> = value.encode_utf16().collect();
        let mut size = windows::Win32::Foundation::SIZE::default();
        let _ = GetTextExtentPoint32W(hdc, &text, &mut size);
        let _ = SelectObject(hdc, old);
        let _ = DeleteObject(HGDIOBJ(font.0));
        size.cx.max(0)
    }
}

unsafe fn create_ui_font(
    locale: Locale,
    height: i32,
    weight: i32,
) -> windows::Win32::Graphics::Gdi::HFONT {
    let face = wide0(ui_font_face(locale));
    unsafe {
        CreateFontW(
            -height,
            0,
            0,
            0,
            weight,
            0,
            0,
            0,
            DEFAULT_CHARSET,
            OUT_DEFAULT_PRECIS,
            CLIP_DEFAULT_PRECIS,
            FONT_QUALITY(CLEARTYPE_QUALITY.0),
            u32::from(DEFAULT_PITCH.0 | FF_SWISS.0),
            PCWSTR(face.as_ptr()),
        )
    }
}

const fn ui_font_face(_locale: Locale) -> &'static str {
    // Request one Windows UI family for every run. Windows font linking
    // supplies CJK glyphs without switching the Latin letters and numbers to a
    // different face for each individual string.
    "Segoe UI Variable Text"
}

unsafe fn fill(hdc: HDC, rect: RECT, color: COLORREF) {
    unsafe {
        let brush = CreateSolidBrush(color);
        let _ = FillRect(hdc, &rect, brush);
        let _ = DeleteObject(HGDIOBJ(brush.0));
    }
}

unsafe fn fill_rounded(hdc: HDC, rect: RECT, radius: i32, color: COLORREF) {
    unsafe {
        let region = CreateRoundRectRgn(
            rect.left,
            rect.top,
            rect.right + 1,
            rect.bottom + 1,
            radius.saturating_mul(2).max(1),
            radius.saturating_mul(2).max(1),
        );
        let brush = CreateSolidBrush(color);
        let _ = FillRgn(hdc, region, brush);
        let _ = DeleteObject(HGDIOBJ(brush.0));
        let _ = DeleteObject(HGDIOBJ(region.0));
    }
}

pub fn scale(value: i32, dpi: u32) -> i32 {
    value * dpi as i32 / 96
}

const fn rgb(red: u8, green: u8, blue: u8) -> COLORREF {
    COLORREF(red as u32 | ((green as u32) << 8) | ((blue as u32) << 16))
}

fn wide0(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{AccountSummary, QuotaSnapshot, QuotaWindow};

    #[test]
    fn localizes_tooltip_status() {
        let state =
            DisplayState { snapshot: None, refresh_state: RefreshState::Unavailable, error: None };
        assert!(tooltip_for_metric(&state, Locale::English, "weekly").contains("unavailable"));
        assert!(tooltip_for_metric(&state, Locale::Chinese, "weekly").contains("不可用"));
    }

    #[test]
    fn reset_countdown_never_goes_negative() {
        let window = QuotaWindow {
            used_percent: 0.0,
            remaining_percent: 100.0,
            window_minutes: 10_080,
            resets_at: Some(1),
        };
        let (countdown, _) = reset_details(&window, Locale::English);
        assert!(!countdown.contains('-'));
    }

    #[test]
    fn uses_one_ui_font_family_in_every_locale() {
        assert_eq!(ui_font_face(Locale::Chinese), ui_font_face(Locale::English));
    }

    #[test]
    fn labels_supported_personal_plans() {
        assert_eq!(plan_label("free", Locale::Chinese), "免费");
        assert_eq!(plan_label("free", Locale::English), "Free");
        assert_eq!(plan_label("go", Locale::Chinese), "Go");
        assert_eq!(plan_label("plus", Locale::Chinese), "Plus");
        assert_eq!(plan_label("prolite", Locale::Chinese), "Pro Lite");
        assert_eq!(plan_label("pro", Locale::Chinese), "Pro");
    }

    #[test]
    fn labels_supported_organization_plans() {
        assert_eq!(plan_label("team", Locale::English), "Business");
        assert_eq!(plan_label("self_serve_business_usage_based", Locale::English), "Business");
        assert_eq!(plan_label("business", Locale::English), "Enterprise");
        assert_eq!(plan_label("ent26", Locale::English), "Enterprise");
        assert_eq!(plan_label("enterprise_cbp_usage_based", Locale::English), "Enterprise");
        assert_eq!(plan_label("edu", Locale::English), "Edu");
        assert_eq!(plan_label("edu", Locale::Chinese), "教育版");
        assert_eq!(plan_label("future_plan", Locale::English), "future_plan");
    }

    fn weekly_state(used_percent: f64, fetched_at: i64, resets_at: Option<i64>) -> DisplayState {
        DisplayState::live(QuotaSnapshot {
            weekly: Some(QuotaWindow {
                used_percent,
                remaining_percent: (100.0 - used_percent).clamp(0.0, 100.0),
                window_minutes: 10_080,
                resets_at,
            }),
            session: None,
            account: AccountSummary::default(),
            fetched_at,
        })
    }

    #[test]
    fn projects_from_the_snapshot_observation_time() {
        let reset_at = 10_080 * 60;
        let fetched_at = 24 * 60 * 60;
        assert_eq!(
            weekly_usage_projection(&weekly_state(10.0, fetched_at, Some(reset_at)), fetched_at),
            None
        );
        assert_eq!(
            weekly_usage_projection(
                &weekly_state(50.0, fetched_at, Some(reset_at)),
                fetched_at + 60 * 60
            ),
            Some(UsageProjection::DepletesIn { seconds: 23 * 60 * 60 })
        );
        assert_eq!(
            weekly_usage_projection(&weekly_state(100.0, fetched_at, None), fetched_at),
            Some(UsageProjection::Exhausted)
        );
    }

    #[test]
    fn projection_keeps_the_existing_ahead_of_pace_threshold() {
        let reset_at = 10_080 * 60;
        let fetched_at = 36 * 60 * 60;
        assert_eq!(
            weekly_usage_projection(&weekly_state(22.0, fetched_at, Some(reset_at)), fetched_at),
            None
        );
        let state = weekly_state(22.0, fetched_at, Some(reset_at));
        let insight = analyze_window(
            state.snapshot.as_ref().and_then(|snapshot| snapshot.weekly.as_ref()).unwrap(),
            fetched_at,
        );
        assert!(insight.likely_exhaust_before_reset);
        assert!(!insight.is_ahead_of_pace);
    }

    #[test]
    fn localizes_projection_labels() {
        assert_eq!(
            projection_label(
                UsageProjection::DepletesIn { seconds: 25 * 60 * 60 },
                Locale::Chinese,
            ),
            ProjectionLabel { text: "用量偏快 · 约1天1小时后耗尽".to_owned() }
        );
        assert_eq!(
            projection_label(
                UsageProjection::DepletesIn { seconds: 25 * 60 * 60 },
                Locale::English,
            ),
            ProjectionLabel { text: "Pace high · empty in ~1d 1h".to_owned() }
        );
        assert_eq!(
            projection_label(UsageProjection::Exhausted, Locale::Chinese),
            ProjectionLabel { text: "额度耗尽".to_owned() }
        );
        assert_eq!(
            projection_label(UsageProjection::Exhausted, Locale::English),
            ProjectionLabel { text: "Quota exhausted".to_owned() }
        );
    }

    #[test]
    fn tray_metric_can_show_weekly_session_or_the_tighter_window() {
        let state = DisplayState::live(QuotaSnapshot {
            weekly: Some(QuotaWindow {
                used_percent: 35.0,
                remaining_percent: 65.0,
                window_minutes: 10_080,
                resets_at: Some(100),
            }),
            session: Some(QuotaWindow {
                used_percent: 78.0,
                remaining_percent: 22.0,
                window_minutes: 300,
                resets_at: Some(100),
            }),
            account: AccountSummary::default(),
            fetched_at: 0,
        });
        assert_eq!(tray_percent(&state, "weekly"), Some(65));
        assert_eq!(tray_percent(&state, "session"), Some(22));
        assert_eq!(tray_percent(&state, "lowest"), Some(22));
    }

    #[test]
    fn refresh_hit_target_scales_with_dpi() {
        assert!(refresh_hit_test(392, 30, 96));
        assert!(!refresh_hit_test(350, 30, 96));
        assert!(refresh_hit_test(784, 60, 192));
    }
}
