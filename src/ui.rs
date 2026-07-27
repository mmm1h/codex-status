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
    DEFAULT_PITCH, DT_END_ELLIPSIS, DT_LEFT, DT_NOPREFIX, DT_RIGHT, DT_SINGLELINE, DT_VCENTER,
    DeleteDC, DeleteObject, DrawTextW, EndPaint, FF_SWISS, FONT_QUALITY, FW_NORMAL, FW_SEMIBOLD,
    FillRect, FillRgn, GetTextExtentPoint32W, HDC, HGDIOBJ, InvalidateRect, OUT_DEFAULT_PRECIS,
    PAINTSTRUCT, SRCCOPY, SelectObject, SetBkMode, SetTextColor, TRANSPARENT,
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
            background: rgb(22, 25, 29),
            surface: rgb(32, 36, 40),
            surface_alt: rgb(39, 44, 48),
            text: rgb(245, 248, 246),
            muted: rgb(168, 177, 172),
            line: rgb(63, 70, 73),
        }
    } else {
        Theme {
            dark,
            tray_dark,
            high_contrast: false,
            background: rgb(243, 244, 240),
            surface: rgb(254, 255, 252),
            surface_alt: rgb(248, 250, 246),
            text: rgb(29, 35, 32),
            muted: rgb(94, 103, 98),
            line: rgb(220, 225, 219),
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

pub fn paint_card(hwnd: HWND, state: &DisplayState, locale: Locale, theme: Theme) {
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
                draw_card(buffer, state, locale, theme, dpi);
                let _ = BitBlt(hdc, 0, 0, width, height, Some(buffer), 0, 0, SRCCOPY);
                let _ = SelectObject(buffer, old_bitmap);
            } else {
                draw_card(hdc, state, locale, theme, dpi);
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

unsafe fn draw_card(hdc: HDC, state: &DisplayState, locale: Locale, theme: Theme, dpi: u32) {
    unsafe {
        let width = scale(CARD_WIDTH, dpi);
        let height = scale(CARD_HEIGHT, dpi);
        fill(hdc, RECT { left: 0, top: 0, right: width, bottom: height }, theme.background);
        let _ = SetBkMode(hdc, TRANSPARENT);

        let status_color = accent_for(state, theme.high_contrast);
        fill_rounded(
            hdc,
            RECT {
                left: scale(20, dpi),
                top: scale(27, dpi),
                right: scale(28, dpi),
                bottom: scale(35, dpi),
            },
            scale(8, dpi),
            status_color,
        );
        draw_text(
            hdc,
            locale,
            "CodexStatus",
            RECT {
                left: scale(39, dpi),
                top: scale(10, dpi),
                right: scale(202, dpi),
                bottom: scale(52, dpi),
            },
            scale(18, dpi),
            FW_SEMIBOLD.0 as i32,
            theme.text,
        );

        draw_text_right(
            hdc,
            locale,
            &updated_text(state, locale),
            RECT {
                left: scale(205, dpi),
                top: scale(11, dpi),
                right: scale(365, dpi),
                bottom: scale(51, dpi),
            },
            scale(12, dpi),
            FW_NORMAL.0 as i32,
            theme.muted,
        );
        draw_refresh_button(hdc, theme, dpi);

        let hero = RECT {
            left: scale(16, dpi),
            top: scale(68, dpi),
            right: width - scale(16, dpi),
            bottom: scale(282, dpi),
        };
        outlined_surface(hdc, hero, scale(19, dpi), theme.surface, theme.line, dpi);

        draw_text(
            hdc,
            locale,
            locale.text("Weekly remaining", "本周剩余"),
            RECT {
                left: scale(34, dpi),
                top: scale(82, dpi),
                right: scale(210, dpi),
                bottom: scale(115, dpi),
            },
            scale(15, dpi),
            FW_NORMAL.0 as i32,
            theme.muted,
        );
        draw_percentage(hdc, state.weekly_percent(), locale, theme, status_color, dpi);

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
        fill(
            hdc,
            RECT {
                left: scale(220, dpi),
                top: scale(91, dpi),
                right: scale(221, dpi),
                bottom: scale(184, dpi),
            },
            theme.line,
        );
        draw_text(
            hdc,
            locale,
            locale.text("Reset in", "距离重置"),
            RECT {
                left: scale(239, dpi),
                top: scale(82, dpi),
                right: scale(385, dpi),
                bottom: scale(115, dpi),
            },
            scale(15, dpi),
            FW_NORMAL.0 as i32,
            theme.muted,
        );
        draw_text(
            hdc,
            locale,
            &reset.0,
            RECT {
                left: scale(239, dpi),
                top: scale(109, dpi),
                right: scale(385, dpi),
                bottom: scale(146, dpi),
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
                left: scale(239, dpi),
                top: scale(143, dpi),
                right: scale(385, dpi),
                bottom: scale(171, dpi),
            },
            scale(12, dpi),
            FW_NORMAL.0 as i32,
            theme.muted,
        );

        let bar = RECT {
            left: scale(34, dpi),
            top: scale(207, dpi),
            right: width - scale(34, dpi),
            bottom: scale(215, dpi),
        };
        fill_rounded(hdc, bar, scale(8, dpi), theme.line);
        if let Some(value) = state.weekly_percent() {
            let filled = (bar.left + (bar.right - bar.left) * i32::from(value) / 100)
                .max(bar.left + (bar.bottom - bar.top));
            if value > 0 {
                fill_rounded(
                    hdc,
                    RECT { right: filled.min(bar.right), ..bar },
                    scale(8, dpi),
                    status_color,
                );
            }
        }
        if let Some(window) = state.snapshot.as_ref().and_then(|snapshot| snapshot.weekly.as_ref())
        {
            let insight = analyze_window(window, Local::now().timestamp());
            if let Some(elapsed) = insight.elapsed_percent {
                let expected_remaining = (100.0 - elapsed).clamp(0.0, 100.0);
                let marker = bar.left
                    + ((bar.right - bar.left) as f64 * expected_remaining / 100.0).round() as i32;
                fill(
                    hdc,
                    RECT {
                        left: marker.saturating_sub(scale(1, dpi)),
                        top: bar.top - scale(3, dpi),
                        right: marker + scale(1, dpi).max(1),
                        bottom: bar.bottom + scale(3, dpi),
                    },
                    theme.text,
                );
                let pace_warning = insight.is_ahead_of_pace && insight.likely_exhaust_before_reset;
                draw_text(
                    hdc,
                    locale,
                    if pace_warning {
                        locale
                            .text("Usage pace high · may run out early", "用量偏快 · 可能提前耗尽")
                    } else {
                        locale.text("Usage pace on track", "用量节奏正常")
                    },
                    RECT {
                        left: scale(34, dpi),
                        top: scale(222, dpi),
                        right: width - scale(34, dpi),
                        bottom: scale(259, dpi),
                    },
                    scale(14, dpi),
                    FW_SEMIBOLD.0 as i32,
                    if pace_warning && !theme.high_contrast {
                        rgb(210, 134, 0)
                    } else {
                        theme.text
                    },
                );
            }
        }

        let session = state
            .snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.session.as_ref())
            .map(|window| format!("{}%", window.display_percent()));
        let plan = state
            .snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.account.plan_type.as_deref())
            .map(plan_label)
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
            top: scale(298, dpi),
            right: width - scale(16, dpi),
            bottom: scale(397, dpi),
        };
        outlined_surface(hdc, metrics, scale(19, dpi), theme.surface_alt, theme.line, dpi);
        if let Some(session) = session {
            for divider in [scale(146, dpi), scale(274, dpi)] {
                draw_metric_divider(hdc, metrics, divider, theme, dpi);
            }
            metric_column(
                hdc,
                locale,
                RECT { right: scale(146, dpi), ..metrics },
                locale.text("Plan", "套餐"),
                &plan,
                theme,
                dpi,
            );
            metric_column(
                hdc,
                locale,
                RECT { left: scale(147, dpi), right: scale(274, dpi), ..metrics },
                locale.text("5-hour", "5 小时"),
                &session,
                theme,
                dpi,
            );
            metric_column(
                hdc,
                locale,
                RECT { left: scale(275, dpi), ..metrics },
                locale.text("Reset credits", "重置机会"),
                &credits,
                theme,
                dpi,
            );
        } else {
            let divider = scale(220, dpi);
            draw_metric_divider(hdc, metrics, divider, theme, dpi);
            metric_column(
                hdc,
                locale,
                RECT { right: divider, ..metrics },
                locale.text("Plan", "套餐"),
                &plan,
                theme,
                dpi,
            );
            metric_column(
                hdc,
                locale,
                RECT { left: divider + scale(1, dpi), ..metrics },
                locale.text("Reset credits", "重置机会"),
                &credits,
                theme,
                dpi,
            );
        }

        fill_rounded(
            hdc,
            RECT {
                left: scale(21, dpi),
                top: scale(412, dpi),
                right: scale(27, dpi),
                bottom: scale(418, dpi),
            },
            scale(6, dpi),
            if state.error.is_some() { rgb(211, 64, 73) } else { status_color },
        );
        let footer = footer_text(state, locale);
        draw_text(
            hdc,
            locale,
            &footer,
            RECT {
                left: scale(34, dpi),
                top: scale(400, dpi),
                right: width - scale(18, dpi),
                bottom: height,
            },
            scale(12, dpi),
            FW_NORMAL.0 as i32,
            if state.error.is_some() { rgb(211, 64, 73) } else { theme.muted },
        );
    }
}

pub fn refresh_hit_test(x: i32, y: i32, dpi: u32) -> bool {
    let rect = refresh_rect(dpi);
    x >= rect.left && x < rect.right && y >= rect.top && y < rect.bottom
}

fn refresh_rect(dpi: u32) -> RECT {
    RECT {
        left: scale(366, dpi),
        top: scale(5, dpi),
        right: scale(418, dpi),
        bottom: scale(56, dpi),
    }
}

unsafe fn draw_refresh_button(hdc: HDC, theme: Theme, dpi: u32) {
    unsafe {
        let rect = RECT {
            left: scale(374, dpi),
            top: scale(11, dpi),
            right: scale(411, dpi),
            bottom: scale(49, dpi),
        };
        outlined_surface(hdc, rect, scale(12, dpi), theme.surface_alt, theme.line, dpi);
        draw_text(
            hdc,
            Locale::English,
            "↻",
            RECT {
                left: rect.left + scale(8, dpi),
                top: rect.top,
                right: rect.right,
                bottom: rect.bottom,
            },
            scale(22, dpi),
            FW_NORMAL.0 as i32,
            theme.muted,
        );
    }
}

unsafe fn draw_metric_divider(hdc: HDC, metrics: RECT, divider: i32, theme: Theme, dpi: u32) {
    unsafe {
        fill(
            hdc,
            RECT {
                left: divider,
                top: metrics.top + scale(13, dpi),
                right: divider + scale(1, dpi).max(1),
                bottom: metrics.bottom - scale(13, dpi),
            },
            theme.line,
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
                    left: scale(34, dpi),
                    top: scale(105, dpi),
                    right: scale(207, dpi),
                    bottom: scale(191, dpi),
                },
                scale(62, dpi),
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
                left: scale(34, dpi),
                top: scale(105, dpi),
                right: scale(207, dpi),
                bottom: scale(191, dpi),
            },
            scale(62, dpi),
            FW_SEMIBOLD.0 as i32,
            accent,
        );
        let number_left = scale(34, dpi);
        let number_width =
            measure_text_width(hdc, locale, &number, scale(62, dpi), FW_SEMIBOLD.0 as i32);
        draw_text(
            hdc,
            locale,
            "%",
            RECT {
                left: number_left + number_width + scale(2, dpi),
                top: scale(125, dpi),
                right: scale(215, dpi),
                bottom: scale(184, dpi),
            },
            scale(28, dpi),
            FW_SEMIBOLD.0 as i32,
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
                left: rect.left + scale(12, dpi),
                top: rect.top + scale(9, dpi),
                right: rect.right - scale(10, dpi),
                bottom: rect.top + scale(43, dpi),
            },
            scale(14, dpi),
            FW_NORMAL.0 as i32,
            theme.muted,
        );
        draw_text(
            hdc,
            locale,
            value,
            RECT {
                left: rect.left + scale(12, dpi),
                top: rect.top + scale(42, dpi),
                right: rect.right - scale(10, dpi),
                bottom: rect.bottom - scale(8, dpi),
            },
            scale(26, dpi),
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

fn updated_text(state: &DisplayState, locale: Locale) -> String {
    if state.refresh_state == RefreshState::Loading {
        return locale.text("Refreshing…", "刷新中…").to_owned();
    }
    let time = state
        .snapshot
        .as_ref()
        .and_then(|snapshot| DateTime::from_timestamp(snapshot.fetched_at, 0))
        .map(|time| time.with_timezone(&Local).format("%H:%M").to_string());
    let prefix = match state.refresh_state {
        RefreshState::Cached => locale.text("Cached", "缓存"),
        RefreshState::Unavailable => locale.text("Unavailable", "不可用"),
        _ => locale.text("Updated", "更新"),
    };
    time.map_or_else(|| prefix.to_owned(), |time| format!("{prefix} {time}"))
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

pub(crate) fn plan_label(plan: &str) -> &str {
    match plan.to_ascii_lowercase().as_str() {
        "free" => "Free",
        "go" => "Go",
        "plus" => "Plus",
        "pro" => "Pro",
        "team" => "Team",
        "business" => "Business",
        "enterprise" => "Enterprise",
        _ => plan,
    }
}

fn accent_for(state: &DisplayState, high_contrast: bool) -> COLORREF {
    if high_contrast {
        return rgb(255, 255, 255);
    }
    match state.weekly_percent() {
        Some(value) if value < 20 => rgb(211, 64, 73),
        Some(value) if value < 50 => rgb(210, 134, 0),
        Some(_) => rgb(18, 196, 105),
        None => rgb(92, 116, 128),
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

unsafe fn draw_text_right(
    hdc: HDC,
    locale: Locale,
    value: &str,
    rect: RECT,
    height: i32,
    weight: i32,
    color: COLORREF,
) {
    let style = TextStyle { height, weight, color };
    unsafe { draw_text_with_alignment(hdc, locale, value, rect, style, DT_RIGHT) }
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
            radius.max(1),
            radius.max(1),
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
