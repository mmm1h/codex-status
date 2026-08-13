use super::{FLYOUT_CORNER_RADIUS, FLYOUT_SHADOW_MARGIN, Theme, scale};
use std::ffi::c_void;
use std::mem::size_of;
use std::slice;
use windows::Win32::Foundation::{COLORREF, HWND, POINT, SIZE};
use windows::Win32::Graphics::Gdi::{
    AC_SRC_ALPHA, AC_SRC_OVER, BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BLENDFUNCTION,
    CreateCompatibleDC, CreateDIBSection, DIB_RGB_COLORS, DeleteDC, DeleteObject, HGDIOBJ,
    SelectObject,
};
use windows::Win32::UI::WindowsAndMessaging::{
    SW_HIDE, SW_SHOWNOACTIVATE, ShowWindow, ULW_ALPHA, UpdateLayeredWindow,
};

pub fn hide(hwnd: HWND) {
    unsafe {
        let _ = ShowWindow(hwnd, SW_HIDE);
    }
}

pub fn update(
    hwnd: HWND,
    panel_x: i32,
    panel_y: i32,
    panel_width: i32,
    panel_height: i32,
    dpi: u32,
    theme: Theme,
) -> windows::core::Result<()> {
    if theme.high_contrast {
        hide(hwnd);
        return Ok(());
    }

    let margin = scale(FLYOUT_SHADOW_MARGIN, dpi);
    let width = panel_width + margin * 2;
    let height = panel_height + margin * 2;
    let bitmap_info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: width,
            biHeight: -height,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };

    unsafe {
        let memory_dc = CreateCompatibleDC(None);
        if memory_dc.is_invalid() {
            return Err(windows::core::Error::from_thread());
        }

        let mut bits = std::ptr::null_mut::<c_void>();
        let bitmap = match CreateDIBSection(
            Some(memory_dc),
            &bitmap_info,
            DIB_RGB_COLORS,
            &mut bits,
            None,
            0,
        ) {
            Ok(bitmap) => bitmap,
            Err(error) => {
                let _ = DeleteDC(memory_dc);
                return Err(error);
            }
        };
        let old_bitmap = SelectObject(memory_dc, HGDIOBJ(bitmap.0));

        let result = (|| {
            if bits.is_null() {
                return Err(windows::core::Error::from_thread());
            }
            let pixels =
                slice::from_raw_parts_mut(bits.cast::<u8>(), (width * height * 4) as usize);
            render_shadow(
                pixels,
                width,
                height,
                margin,
                panel_width,
                panel_height,
                scale(FLYOUT_CORNER_RADIUS, dpi) as f32,
                dpi,
                theme.dark,
            );

            let destination = POINT { x: panel_x - margin, y: panel_y - margin };
            let size = SIZE { cx: width, cy: height };
            let source = POINT::default();
            let blend = BLENDFUNCTION {
                BlendOp: AC_SRC_OVER as u8,
                BlendFlags: 0,
                SourceConstantAlpha: 255,
                AlphaFormat: AC_SRC_ALPHA as u8,
            };
            UpdateLayeredWindow(
                hwnd,
                None,
                Some(&destination),
                Some(&size),
                Some(memory_dc),
                Some(&source),
                COLORREF(0),
                Some(&blend),
                ULW_ALPHA,
            )?;
            let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
            Ok(())
        })();

        let _ = SelectObject(memory_dc, old_bitmap);
        let _ = DeleteObject(HGDIOBJ(bitmap.0));
        let _ = DeleteDC(memory_dc);
        result
    }
}

#[allow(clippy::too_many_arguments)]
fn render_shadow(
    pixels: &mut [u8],
    width: i32,
    height: i32,
    margin: i32,
    panel_width: i32,
    panel_height: i32,
    radius: f32,
    dpi: u32,
    dark: bool,
) {
    let dpi_scale = dpi as f32 / 96.0;
    let left = margin as f32;
    let top = margin as f32;
    let right = left + panel_width as f32;
    let bottom = top + panel_height as f32;
    let ambient_sigma = 10.5 * dpi_scale;
    let contact_sigma = 5.5 * dpi_scale;
    let ambient_offset = 1.5 * dpi_scale;
    let contact_offset = 5.0 * dpi_scale;
    let (ambient_strength, contact_strength) = if dark { (0.18, 0.14) } else { (0.15, 0.12) };

    for y in 0..height {
        for x in 0..width {
            let sample_x = x as f32 + 0.5;
            let sample_y = y as f32 + 0.5;
            let panel_distance =
                rounded_rect_distance(sample_x, sample_y, left, top, right, bottom, radius);
            let exterior_coverage = smoothstep(-0.75, 0.75, panel_distance);
            let ambient_distance = rounded_rect_distance(
                sample_x,
                sample_y,
                left,
                top + ambient_offset,
                right,
                bottom + ambient_offset,
                radius,
            )
            .max(0.0);
            let contact_distance = rounded_rect_distance(
                sample_x,
                sample_y,
                left,
                top + contact_offset,
                right,
                bottom + contact_offset,
                radius,
            )
            .max(0.0);
            let ambient = gaussian(ambient_distance, ambient_sigma) * ambient_strength;
            let contact = gaussian(contact_distance, contact_sigma) * contact_strength;
            let alpha = (1.0 - (1.0 - ambient) * (1.0 - contact)) * exterior_coverage;
            let offset = ((y * width + x) * 4) as usize;
            pixels[offset] = 0;
            pixels[offset + 1] = 0;
            pixels[offset + 2] = 0;
            pixels[offset + 3] = (alpha.clamp(0.0, 1.0) * 255.0).round() as u8;
        }
    }
}

fn rounded_rect_distance(
    x: f32,
    y: f32,
    left: f32,
    top: f32,
    right: f32,
    bottom: f32,
    radius: f32,
) -> f32 {
    let half_width = (right - left) * 0.5;
    let half_height = (bottom - top) * 0.5;
    let center_x = (left + right) * 0.5;
    let center_y = (top + bottom) * 0.5;
    let qx = (x - center_x).abs() - (half_width - radius);
    let qy = (y - center_y).abs() - (half_height - radius);
    qx.max(0.0).hypot(qy.max(0.0)) + qx.max(qy).min(0.0) - radius
}

fn gaussian(distance: f32, sigma: f32) -> f32 {
    (-0.5 * (distance / sigma).powi(2)).exp()
}

fn smoothstep(edge0: f32, edge1: f32, value: f32) -> f32 {
    let t = ((value - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rounded_rect_distance_is_negative_inside_and_positive_outside() {
        assert!(rounded_rect_distance(50.0, 50.0, 0.0, 0.0, 100.0, 100.0, 20.0) < 0.0);
        assert!(rounded_rect_distance(-10.0, 50.0, 0.0, 0.0, 100.0, 100.0, 20.0) > 0.0);
        assert!(rounded_rect_distance(0.0, 0.0, 0.0, 0.0, 100.0, 100.0, 20.0) > 0.0);
    }

    #[test]
    fn gaussian_fades_monotonically() {
        assert!(gaussian(0.0, 10.0) > gaussian(5.0, 10.0));
        assert!(gaussian(5.0, 10.0) > gaussian(15.0, 10.0));
    }
}
