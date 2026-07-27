use crate::model::{DisplayState, RefreshState};
use std::ffi::c_void;
use std::mem::size_of;
use std::ptr;
use windows::Win32::Foundation::{COLORREF, HINSTANCE, RECT};
use windows::Win32::Graphics::Gdi::{
    ANTIALIASED_QUALITY, BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CLIP_DEFAULT_PRECIS,
    CreateCompatibleDC, CreateDIBSection, CreateFontW, DEFAULT_CHARSET, DEFAULT_PITCH,
    DIB_RGB_COLORS, DT_LEFT, DT_NOPREFIX, DT_SINGLELINE, DT_TOP, DeleteDC, DeleteObject, DrawTextW,
    FF_SWISS, FW_SEMIBOLD, GdiFlush, HGDIOBJ, OPAQUE, OUT_DEFAULT_PRECIS, SelectObject, SetBkColor,
    SetBkMode, SetTextColor,
};
use windows::Win32::UI::WindowsAndMessaging::{CreateIcon, DestroyIcon, HICON};
use windows::core::w;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IconTone {
    Healthy,
    Warning,
    Critical,
    Stale,
    Unavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceOverlay {
    None,
    Degraded,
    Outage,
}

impl IconTone {
    fn accent(self, high_contrast: bool, dark_taskbar: bool) -> [u8; 4] {
        if high_contrast {
            return foreground(dark_taskbar, false);
        }
        match self {
            Self::Healthy => [20, 158, 124, 255],
            Self::Warning => [184, 119, 31, 255],
            Self::Critical => [194, 83, 94, 255],
            Self::Stale => [112, 122, 134, 255],
            Self::Unavailable => [132, 141, 151, 255],
        }
    }

    fn is_muted(self) -> bool {
        matches!(self, Self::Stale | Self::Unavailable)
    }
}

pub fn tone_for_percent(state: &DisplayState, percent: Option<u8>) -> IconTone {
    if state.refresh_state != RefreshState::Live {
        return if percent.is_some() { IconTone::Stale } else { IconTone::Unavailable };
    }
    match percent {
        Some(percent) if percent < 20 => IconTone::Critical,
        Some(percent) if percent < 50 => IconTone::Warning,
        Some(_) => IconTone::Healthy,
        None => IconTone::Unavailable,
    }
}

pub struct OwnedIcon(HICON);

impl OwnedIcon {
    pub fn handle(&self) -> HICON {
        self.0
    }
}

impl Drop for OwnedIcon {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyIcon(self.0);
        }
    }
}

pub fn create_icon_with_overlay(
    percent: Option<u8>,
    tone: IconTone,
    overlay: ServiceOverlay,
    size: u32,
    high_contrast: bool,
    dark_taskbar: bool,
) -> windows::core::Result<OwnedIcon> {
    let size = size.clamp(16, 32);
    let xor = render_bgra_with_overlay(percent, tone, overlay, size, high_contrast, dark_taskbar);
    let mask_stride = size.div_ceil(32) * 4;
    let and_mask = vec![0_u8; (mask_stride * size) as usize];
    let icon = unsafe {
        CreateIcon(
            None::<HINSTANCE>,
            size as i32,
            size as i32,
            1,
            32,
            and_mask.as_ptr(),
            xor.as_ptr(),
        )?
    };
    Ok(OwnedIcon(icon))
}

pub fn render_bgra_with_overlay(
    percent: Option<u8>,
    tone: IconTone,
    overlay: ServiceOverlay,
    size: u32,
    high_contrast: bool,
    dark_taskbar: bool,
) -> Vec<u8> {
    let size = size.clamp(16, 32);
    let pixels = render_rgba(percent, tone, overlay, size, high_contrast, dark_taskbar);
    let mut bytes = Vec::with_capacity((size * size * 4) as usize);

    // CreateIcon's 32-bpp XOR input is consumed in the same top-left order as
    // our logical canvas. Reversing scan lines here made a 2 look like a 5 and
    // moved the status rule above the number in Explorer.
    for y in 0..size {
        for x in 0..size {
            let [r, g, b, a] = pixels[(y * size + x) as usize];
            bytes.extend_from_slice(&[b, g, r, a]);
        }
    }
    bytes
}

fn render_rgba(
    percent: Option<u8>,
    tone: IconTone,
    overlay: ServiceOverlay,
    size: u32,
    high_contrast: bool,
    dark_taskbar: bool,
) -> Vec<[u8; 4]> {
    let mut pixels = vec![[0_u8; 4]; (size * size) as usize];
    let label = percent.map_or_else(|| "--".to_owned(), |value| value.min(100).to_string());
    let text = foreground(dark_taskbar, tone.is_muted() && !high_contrast);

    if let Some(mask) = rasterize_label(&label, size) {
        composite_mask(&mut pixels, size, &mask, text);
    } else {
        draw_fallback_label(&mut pixels, size, &label, text);
    }

    let accent = tone.accent(high_contrast, dark_taskbar);
    let rule_thickness = (size / 8).clamp(2, 3);
    for y in size.saturating_sub(rule_thickness)..size {
        for x in 1..size.saturating_sub(1) {
            set_pixel(&mut pixels, size, x as i32, y as i32, accent);
        }
    }
    draw_service_overlay(&mut pixels, size, overlay, high_contrast, dark_taskbar);
    pixels
}

fn draw_service_overlay(
    pixels: &mut [[u8; 4]],
    size: u32,
    overlay: ServiceOverlay,
    high_contrast: bool,
    dark_taskbar: bool,
) {
    if overlay == ServiceOverlay::None {
        return;
    }
    let scale = (size / 16).max(1) as i32;
    let color = if high_contrast {
        foreground(dark_taskbar, false)
    } else if overlay == ServiceOverlay::Outage {
        [194, 83, 94, 255]
    } else {
        [184, 119, 31, 255]
    };
    let right = size as i32 - 1;
    for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
        for sy in 0..scale {
            for sx in 0..scale {
                set_pixel(pixels, size, right - (dx + 1) * scale + sx, dy * scale + sy, color);
            }
        }
    }
}

fn foreground(dark_taskbar: bool, muted: bool) -> [u8; 4] {
    match (dark_taskbar, muted) {
        (true, true) => [183, 190, 199, 255],
        (true, false) => [248, 249, 250, 255],
        (false, true) => [94, 103, 114, 255],
        (false, false) => [31, 34, 38, 255],
    }
}

struct GrayMask {
    pixels: Vec<u8>,
    width: usize,
    height: usize,
}

fn rasterize_label(label: &str, target_size: u32) -> Option<GrayMask> {
    const SUPERSAMPLE: i32 = 8;
    let canvas_height = target_size as i32 * SUPERSAMPLE;
    let canvas_width = target_size as i32 * SUPERSAMPLE * 2;
    let bitmap_info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: canvas_width,
            biHeight: -canvas_height,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut bits = ptr::null_mut::<c_void>();

    unsafe {
        let dc = CreateCompatibleDC(None);
        if dc.0.is_null() {
            return None;
        }
        let bitmap =
            match CreateDIBSection(Some(dc), &bitmap_info, DIB_RGB_COLORS, &mut bits, None, 0) {
                Ok(bitmap) => bitmap,
                Err(_) => {
                    let _ = DeleteDC(dc);
                    return None;
                }
            };
        if bits.is_null() {
            let _ = DeleteObject(bitmap.into());
            let _ = DeleteDC(dc);
            return None;
        }

        let font = CreateFontW(
            -(canvas_height * 7 / 8),
            0,
            0,
            0,
            FW_SEMIBOLD.0 as i32,
            0,
            0,
            0,
            DEFAULT_CHARSET,
            OUT_DEFAULT_PRECIS,
            CLIP_DEFAULT_PRECIS,
            ANTIALIASED_QUALITY,
            (DEFAULT_PITCH.0 | FF_SWISS.0) as u32,
            w!("Segoe UI"),
        );
        if font.0.is_null() {
            let _ = DeleteObject(bitmap.into());
            let _ = DeleteDC(dc);
            return None;
        }

        let old_bitmap = SelectObject(dc, bitmap.into());
        let old_font = SelectObject(dc, font.into());
        let _ = SetBkMode(dc, OPAQUE);
        let _ = SetBkColor(dc, COLORREF(0));
        let _ = SetTextColor(dc, COLORREF(0x00ff_ffff));
        let mut text: Vec<u16> = label.encode_utf16().collect();
        let mut rect = RECT { left: 0, top: 0, right: canvas_width, bottom: canvas_height };
        let _ = DrawTextW(dc, &mut text, &mut rect, DT_LEFT | DT_TOP | DT_SINGLELINE | DT_NOPREFIX);
        let _ = GdiFlush();

        let source = std::slice::from_raw_parts(
            bits.cast::<u8>(),
            (canvas_width * canvas_height * 4) as usize,
        );
        let result = crop_grayscale(source, canvas_width as usize, canvas_height as usize);

        if !old_font.0.is_null() {
            let _ = SelectObject(dc, old_font);
        }
        if !old_bitmap.0.is_null() {
            let _ = SelectObject(dc, old_bitmap);
        }
        let _ = DeleteObject(HGDIOBJ(font.0));
        let _ = DeleteObject(HGDIOBJ(bitmap.0));
        let _ = DeleteDC(dc);
        result
    }
}

fn crop_grayscale(source: &[u8], width: usize, height: usize) -> Option<GrayMask> {
    let mut left = width;
    let mut top = height;
    let mut right = 0;
    let mut bottom = 0;
    for y in 0..height {
        for x in 0..width {
            let pixel = &source[(y * width + x) * 4..][..3];
            let value = *pixel.iter().max().unwrap_or(&0);
            if value <= 3 {
                continue;
            }
            left = left.min(x);
            top = top.min(y);
            right = right.max(x + 1);
            bottom = bottom.max(y + 1);
        }
    }
    if left >= right || top >= bottom {
        return None;
    }

    let cropped_width = right - left;
    let cropped_height = bottom - top;
    let mut pixels = vec![0_u8; cropped_width * cropped_height];
    for y in 0..cropped_height {
        for x in 0..cropped_width {
            let source_index = ((top + y) * width + left + x) * 4;
            pixels[y * cropped_width + x] =
                source[source_index..source_index + 3].iter().copied().max().unwrap_or(0);
        }
    }
    Some(GrayMask { pixels, width: cropped_width, height: cropped_height })
}

fn composite_mask(pixels: &mut [[u8; 4]], size: u32, mask: &GrayMask, color: [u8; 4]) {
    let rule_thickness = (size / 8).clamp(2, 3);
    let max_width = size as usize;
    let max_height = size.saturating_sub(rule_thickness + 1) as usize;
    let width_limited_height = max_width * mask.height / mask.width.max(1);
    let target_height = max_height.min(width_limited_height.max(1));
    let target_width = (target_height * mask.width / mask.height.max(1)).clamp(1, max_width);
    let origin_x = (size as usize - target_width) / 2;
    let origin_y = (max_height.saturating_sub(target_height)) / 2;

    for y in 0..target_height {
        for x in 0..target_width {
            let source_x0 = x * mask.width / target_width;
            let source_x1 = ((x + 1) * mask.width / target_width).max(source_x0 + 1);
            let source_y0 = y * mask.height / target_height;
            let source_y1 = ((y + 1) * mask.height / target_height).max(source_y0 + 1);
            let mut coverage = 0_u32;
            let mut samples = 0_u32;
            for source_y in source_y0..source_y1.min(mask.height) {
                for source_x in source_x0..source_x1.min(mask.width) {
                    coverage += u32::from(mask.pixels[source_y * mask.width + source_x]);
                    samples += 1;
                }
            }
            let alpha = coverage.checked_div(samples).unwrap_or(0) as u8;
            if alpha <= 5 {
                continue;
            }
            set_pixel(
                pixels,
                size,
                (origin_x + x) as i32,
                (origin_y + y) as i32,
                premultiply(color, alpha),
            );
        }
    }
}

fn premultiply(color: [u8; 4], alpha: u8) -> [u8; 4] {
    let scale = u16::from(alpha);
    [
        ((u16::from(color[0]) * scale + 127) / 255) as u8,
        ((u16::from(color[1]) * scale + 127) / 255) as u8,
        ((u16::from(color[2]) * scale + 127) / 255) as u8,
        alpha,
    ]
}

fn draw_fallback_label(pixels: &mut [[u8; 4]], size: u32, label: &str, color: [u8; 4]) {
    let glyph_width = 3_i32;
    let glyph_height = 7_i32;
    let glyph_count = label.chars().count() as i32;
    let units_width = glyph_count * glyph_width + (glyph_count - 1).max(0);
    let rule_thickness = (size / 8).clamp(2, 3) as i32;
    let available_height = size as i32 - rule_thickness - 1;
    let scale_x = ((size as i32 - 1) / units_width).max(1);
    let scale_y = (available_height / glyph_height).max(1);
    let width = units_width * scale_x;
    let height = glyph_height * scale_y;
    let origin_x = (size as i32 - width) / 2;
    let origin_y = (available_height - height) / 2;

    for (index, character) in label.chars().enumerate() {
        let rows = fallback_glyph(character);
        let offset_x = origin_x + index as i32 * (glyph_width + 1) * scale_x;
        for (row, bits) in rows.iter().enumerate() {
            for column in 0..glyph_width {
                if bits & (1 << (glyph_width - 1 - column)) == 0 {
                    continue;
                }
                for dy in 0..scale_y {
                    for dx in 0..scale_x {
                        set_pixel(
                            pixels,
                            size,
                            offset_x + column * scale_x + dx,
                            origin_y + row as i32 * scale_y + dy,
                            color,
                        );
                    }
                }
            }
        }
    }
}

fn set_pixel(pixels: &mut [[u8; 4]], size: u32, x: i32, y: i32, color: [u8; 4]) {
    if x < 0 || y < 0 || x >= size as i32 || y >= size as i32 {
        return;
    }
    pixels[(y as u32 * size + x as u32) as usize] = color;
}

fn fallback_glyph(character: char) -> [u8; 7] {
    match character {
        '0' => [0b111, 0b101, 0b101, 0b101, 0b101, 0b101, 0b111],
        '1' => [0b010, 0b110, 0b010, 0b010, 0b010, 0b010, 0b111],
        '2' => [0b111, 0b001, 0b001, 0b111, 0b100, 0b100, 0b111],
        '3' => [0b111, 0b001, 0b001, 0b111, 0b001, 0b001, 0b111],
        '4' => [0b101, 0b101, 0b101, 0b111, 0b001, 0b001, 0b001],
        '5' => [0b111, 0b100, 0b100, 0b111, 0b001, 0b001, 0b111],
        '6' => [0b111, 0b100, 0b100, 0b111, 0b101, 0b101, 0b111],
        '7' => [0b111, 0b001, 0b001, 0b010, 0b010, 0b010, 0b010],
        '8' => [0b111, 0b101, 0b101, 0b111, 0b101, 0b101, 0b111],
        '9' => [0b111, 0b101, 0b101, 0b111, 0b001, 0b001, 0b111],
        '-' => [0b000, 0b000, 0b000, 0b111, 0b000, 0b000, 0b000],
        _ => [0; 7],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_all_boundary_labels_at_supported_sizes_and_themes() {
        for size in [16, 20, 24, 32] {
            for dark in [false, true] {
                for value in
                    [None, Some(0), Some(9), Some(20), Some(49), Some(50), Some(87), Some(100)]
                {
                    let pixels = render_bgra_with_overlay(
                        value,
                        IconTone::Healthy,
                        ServiceOverlay::None,
                        size,
                        false,
                        dark,
                    );
                    assert_eq!(pixels.len(), (size * size * 4) as usize);
                    assert!(pixels.iter().any(|byte| *byte != 0));
                }
            }
        }
    }

    #[test]
    fn common_two_digit_value_is_readable_but_keeps_a_transparent_background() {
        let pixels =
            render_rgba(Some(87), IconTone::Healthy, ServiceOverlay::None, 16, false, false);
        let visible: Vec<_> =
            pixels[..14 * 16].iter().enumerate().filter(|(_, pixel)| pixel[3] > 20).collect();
        let left = visible.iter().map(|(index, _)| index % 16).min().unwrap();
        let right = visible.iter().map(|(index, _)| index % 16).max().unwrap();
        assert!(right - left + 1 >= 14);
        assert!(pixels.iter().filter(|pixel| pixel[3] == 0).count() > 16 * 5);
    }

    #[test]
    fn light_and_dark_taskbars_get_opposite_foreground_colors() {
        let light =
            render_rgba(Some(87), IconTone::Healthy, ServiceOverlay::None, 16, false, false);
        let dark = render_rgba(Some(87), IconTone::Healthy, ServiceOverlay::None, 16, false, true);
        let sample = light
            .iter()
            .zip(&dark)
            .find(|(light, dark)| light[3] > 200 && dark[3] > 200 && light[0] != 16)
            .unwrap();
        assert!(sample.0[0] < sample.1[0]);
    }

    #[test]
    fn status_rule_is_two_pixels_at_16_and_three_at_larger_sizes() {
        for (size, thickness) in [(16, 2), (24, 3), (32, 3)] {
            let pixels =
                render_rgba(Some(50), IconTone::Healthy, ServiceOverlay::None, size, false, false);
            let accent = IconTone::Healthy.accent(false, false);
            for y in size - thickness..size {
                let row = &pixels[(y * size + 1) as usize..((y + 1) * size - 1) as usize];
                assert!(row.iter().all(|pixel| *pixel == accent));
            }
            let row_above = &pixels
                [((size - thickness - 1) * size) as usize..((size - thickness) * size) as usize];
            assert!(!row_above.contains(&accent));
        }
    }

    #[test]
    fn create_icon_bytes_keep_the_status_rule_on_the_bottom() {
        let pixels = render_bgra_with_overlay(
            Some(50),
            IconTone::Healthy,
            ServiceOverlay::None,
            16,
            false,
            false,
        );
        let accent = IconTone::Healthy.accent(false, false);
        let accent_bgra = [accent[2], accent[1], accent[0], accent[3]];
        let pixel = |x: usize, y: usize| &pixels[(y * 16 + x) * 4..][..4];
        assert_ne!(pixel(8, 0), accent_bgra);
        assert_eq!(pixel(8, 15), accent_bgra);
    }

    #[test]
    fn service_overlay_uses_a_small_top_right_badge_without_moving_the_quota_rule() {
        let plain =
            render_rgba(Some(87), IconTone::Healthy, ServiceOverlay::None, 16, false, false);
        let outage =
            render_rgba(Some(87), IconTone::Healthy, ServiceOverlay::Outage, 16, false, false);
        for y in 0..2 {
            for x in 13..15 {
                assert_eq!(outage[y * 16 + x], [194, 83, 94, 255]);
                assert_ne!(outage[y * 16 + x], plain[y * 16 + x]);
            }
        }
        assert_eq!(&plain[15 * 16..16 * 16], &outage[15 * 16..16 * 16]);
    }

    #[test]
    fn classifies_thresholds_without_relying_on_color_alone() {
        fn state(percent: u8) -> DisplayState {
            use crate::model::{AccountSummary, QuotaSnapshot, QuotaWindow, WEEK_MINUTES};
            DisplayState {
                snapshot: Some(QuotaSnapshot {
                    weekly: Some(QuotaWindow {
                        used_percent: 100.0 - f64::from(percent),
                        remaining_percent: f64::from(percent),
                        window_minutes: WEEK_MINUTES,
                        resets_at: Some(i64::MAX),
                    }),
                    session: None,
                    account: AccountSummary::default(),
                    fetched_at: 0,
                }),
                refresh_state: RefreshState::Live,
                error: None,
            }
        }
        for (percent, expected) in [
            (19, IconTone::Critical),
            (20, IconTone::Warning),
            (49, IconTone::Warning),
            (50, IconTone::Healthy),
        ] {
            let display = state(percent);
            assert_eq!(tone_for_percent(&display, display.weekly_percent()), expected);
        }
    }
}
