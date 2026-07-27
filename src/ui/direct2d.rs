//! Direct2D/DirectWrite renderer for the compact quota flyout.
//!
//! The Win32 host, tray icon, menus, and input behavior remain native. This
//! module owns only the flyout pixels and is deliberately lazy so the graphics
//! stack is not initialized until the card is first shown.

use super::{
    CardDecorations, Locale, ServiceHealth, Theme, accent_for, footer_text, plan_label,
    reset_details, rgb, updated_text,
};
use crate::insights::analyze_window;
use crate::model::{DisplayState, RefreshState};
use chrono::Local;
use std::cell::RefCell;
use windows::Win32::Foundation::{COLORREF, D2DERR_RECREATE_TARGET, HWND};
use windows::Win32::Graphics::Direct2D::Common::{
    D2D_RECT_F, D2D_SIZE_U, D2D1_ALPHA_MODE_UNKNOWN, D2D1_COLOR_F, D2D1_GRADIENT_STOP,
    D2D1_PIXEL_FORMAT,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1_ANTIALIAS_MODE_PER_PRIMITIVE, D2D1_DRAW_TEXT_OPTIONS_NONE, D2D1_EXTEND_MODE_CLAMP,
    D2D1_FACTORY_TYPE_SINGLE_THREADED, D2D1_FEATURE_LEVEL_DEFAULT, D2D1_GAMMA_2_2,
    D2D1_HWND_RENDER_TARGET_PROPERTIES, D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES,
    D2D1_PRESENT_OPTIONS_NONE, D2D1_RENDER_TARGET_PROPERTIES, D2D1_RENDER_TARGET_TYPE_DEFAULT,
    D2D1_RENDER_TARGET_USAGE_NONE, D2D1_ROUNDED_RECT, D2D1_TEXT_ANTIALIAS_MODE_CLEARTYPE,
    D2D1CreateFactory, ID2D1Factory, ID2D1HwndRenderTarget, ID2D1LinearGradientBrush,
    ID2D1SolidColorBrush, ID2D1StrokeStyle,
};
use windows::Win32::Graphics::DirectWrite::{
    DWRITE_FACTORY_TYPE_SHARED, DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE_NORMAL,
    DWRITE_FONT_WEIGHT, DWRITE_FONT_WEIGHT_NORMAL, DWRITE_FONT_WEIGHT_SEMI_BOLD,
    DWRITE_MEASURING_MODE_NATURAL, DWRITE_PARAGRAPH_ALIGNMENT_CENTER, DWRITE_TEXT_ALIGNMENT,
    DWRITE_TEXT_ALIGNMENT_LEADING, DWRITE_TEXT_ALIGNMENT_TRAILING, DWRITE_TEXT_METRICS,
    DWRITE_TRIMMING, DWRITE_TRIMMING_GRANULARITY_CHARACTER, DWRITE_WORD_WRAPPING_NO_WRAP,
    DWriteCreateFactory, IDWriteFactory, IDWriteFontCollection, IDWriteTextFormat,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_UNKNOWN;
use windows::core::{BOOL, PCWSTR, Result, w};
use windows_numerics::Vector2;

thread_local! {
    static RENDERER: RefCell<Option<Renderer>> = const { RefCell::new(None) };
}

pub(super) struct PaintInput<'a> {
    pub hwnd: HWND,
    pub size: (i32, i32),
    pub dpi: u32,
    pub state: &'a DisplayState,
    pub locale: Locale,
    pub theme: Theme,
    pub decorations: CardDecorations,
}

pub(super) fn paint(input: PaintInput<'_>) -> bool {
    RENDERER.with(|slot| {
        let mut slot = slot.borrow_mut();
        if slot.is_none() {
            match Renderer::new() {
                Ok(renderer) => *slot = Some(renderer),
                Err(error) => {
                    diagnostic_failure("initialize", &error);
                    return false;
                }
            }
        }

        let result = slot.as_mut().expect("renderer initialized").paint(&input);
        if let Err(error) = &result {
            diagnostic_failure("paint", error);
            if let Some(renderer) = slot.as_mut() {
                renderer.target = None;
            }
        }
        result.is_ok()
    })
}

pub(super) fn release() {
    RENDERER.with(|slot| *slot.borrow_mut() = None);
}

#[cfg(feature = "diagnostics")]
fn diagnostic_failure(stage: &str, error: &windows::core::Error) {
    eprintln!("Direct2D {stage} failed: {:#x}", error.code().0);
}

#[cfg(not(feature = "diagnostics"))]
fn diagnostic_failure(_stage: &str, _error: &windows::core::Error) {}

struct Renderer {
    factory: ID2D1Factory,
    dwrite: IDWriteFactory,
    font_family: PCWSTR,
    target: Option<ID2D1HwndRenderTarget>,
    pixel_size: D2D_SIZE_U,
    dpi: u32,
    formats: Option<FormatSet>,
}

impl Renderer {
    fn new() -> Result<Self> {
        unsafe {
            let factory =
                D2D1CreateFactory::<ID2D1Factory>(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
            let dwrite = DWriteCreateFactory::<IDWriteFactory>(DWRITE_FACTORY_TYPE_SHARED)?;
            let font_family = select_font_family(&dwrite);
            Ok(Self {
                factory,
                dwrite,
                font_family,
                target: None,
                pixel_size: D2D_SIZE_U::default(),
                dpi: 0,
                formats: None,
            })
        }
    }

    fn paint(&mut self, input: &PaintInput<'_>) -> Result<()> {
        self.ensure_target(input.hwnd, input.size.0, input.size.1, input.dpi)?;
        self.ensure_formats(input.locale)?;

        let target = self.target.as_ref().expect("target initialized").clone();
        let formats = self.formats.as_ref().expect("formats initialized").clone();
        let dwrite = self.dwrite.clone();

        unsafe { target.BeginDraw() };
        let draw_result = draw_frame(
            &target,
            &dwrite,
            &formats,
            input.state,
            input.locale,
            input.theme,
            input.decorations,
        );
        let end_result = unsafe { target.EndDraw(None, None) };
        if end_result.as_ref().is_err_and(|error| error.code() == D2DERR_RECREATE_TARGET) {
            self.target = None;
        }
        draw_result?;
        end_result
    }

    fn ensure_target(&mut self, hwnd: HWND, width: i32, height: i32, dpi: u32) -> Result<()> {
        let size = D2D_SIZE_U { width: width.max(1) as u32, height: height.max(1) as u32 };
        let wrong_window =
            self.target.as_ref().is_some_and(|target| unsafe { target.GetHwnd() != hwnd });
        if wrong_window {
            self.target = None;
        }

        if self.target.is_none() {
            let properties = D2D1_RENDER_TARGET_PROPERTIES {
                r#type: D2D1_RENDER_TARGET_TYPE_DEFAULT,
                pixelFormat: D2D1_PIXEL_FORMAT {
                    format: DXGI_FORMAT_UNKNOWN,
                    alphaMode: D2D1_ALPHA_MODE_UNKNOWN,
                },
                dpiX: dpi as f32,
                dpiY: dpi as f32,
                usage: D2D1_RENDER_TARGET_USAGE_NONE,
                minLevel: D2D1_FEATURE_LEVEL_DEFAULT,
            };
            let hwnd_properties = D2D1_HWND_RENDER_TARGET_PROPERTIES {
                hwnd,
                pixelSize: size,
                presentOptions: D2D1_PRESENT_OPTIONS_NONE,
            };
            let target =
                unsafe { self.factory.CreateHwndRenderTarget(&properties, &hwnd_properties)? };
            unsafe {
                target.SetAntialiasMode(D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
                target.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_CLEARTYPE);
            }
            self.target = Some(target);
            self.pixel_size = size;
            self.dpi = dpi;
            return Ok(());
        }

        let target = self.target.as_ref().expect("target initialized");
        if size != self.pixel_size {
            unsafe { target.Resize(&size)? };
            self.pixel_size = size;
        }
        if dpi != self.dpi {
            unsafe { target.SetDpi(dpi as f32, dpi as f32) };
            self.dpi = dpi;
        }
        Ok(())
    }

    fn ensure_formats(&mut self, locale: Locale) -> Result<()> {
        if self.formats.as_ref().is_none_or(|formats| formats.locale != locale) {
            self.formats = Some(FormatSet::new(&self.dwrite, self.font_family, locale)?);
        }
        Ok(())
    }
}

#[derive(Clone)]
struct FormatSet {
    locale: Locale,
    header: IDWriteTextFormat,
    update: IDWriteTextFormat,
    label: IDWriteTextFormat,
    quota: IDWriteTextFormat,
    percent: IDWriteTextFormat,
    value: IDWriteTextFormat,
    secondary: IDWriteTextFormat,
    metric_label: IDWriteTextFormat,
    metric_value: IDWriteTextFormat,
    footer: IDWriteTextFormat,
}

impl FormatSet {
    fn new(factory: &IDWriteFactory, family: PCWSTR, locale: Locale) -> Result<Self> {
        Ok(Self {
            locale,
            header: make_format(
                factory,
                family,
                locale,
                14.0,
                DWRITE_FONT_WEIGHT_SEMI_BOLD,
                DWRITE_TEXT_ALIGNMENT_LEADING,
                false,
            )?,
            update: make_format(
                factory,
                family,
                locale,
                10.5,
                DWRITE_FONT_WEIGHT_NORMAL,
                DWRITE_TEXT_ALIGNMENT_TRAILING,
                true,
            )?,
            label: make_format(
                factory,
                family,
                locale,
                11.5,
                DWRITE_FONT_WEIGHT_SEMI_BOLD,
                DWRITE_TEXT_ALIGNMENT_LEADING,
                true,
            )?,
            quota: make_format(
                factory,
                family,
                locale,
                42.0,
                DWRITE_FONT_WEIGHT_SEMI_BOLD,
                DWRITE_TEXT_ALIGNMENT_LEADING,
                false,
            )?,
            percent: make_format(
                factory,
                family,
                locale,
                17.0,
                DWRITE_FONT_WEIGHT_SEMI_BOLD,
                DWRITE_TEXT_ALIGNMENT_LEADING,
                false,
            )?,
            value: make_format(
                factory,
                family,
                locale,
                17.5,
                DWRITE_FONT_WEIGHT_SEMI_BOLD,
                DWRITE_TEXT_ALIGNMENT_LEADING,
                true,
            )?,
            secondary: make_format(
                factory,
                family,
                locale,
                10.5,
                DWRITE_FONT_WEIGHT_NORMAL,
                DWRITE_TEXT_ALIGNMENT_LEADING,
                true,
            )?,
            metric_label: make_format(
                factory,
                family,
                locale,
                10.5,
                DWRITE_FONT_WEIGHT_NORMAL,
                DWRITE_TEXT_ALIGNMENT_LEADING,
                true,
            )?,
            metric_value: make_format(
                factory,
                family,
                locale,
                17.0,
                DWRITE_FONT_WEIGHT_SEMI_BOLD,
                DWRITE_TEXT_ALIGNMENT_LEADING,
                true,
            )?,
            footer: make_format(
                factory,
                family,
                locale,
                10.5,
                DWRITE_FONT_WEIGHT_NORMAL,
                DWRITE_TEXT_ALIGNMENT_LEADING,
                true,
            )?,
        })
    }
}

#[allow(clippy::too_many_arguments)]
fn make_format(
    factory: &IDWriteFactory,
    family: PCWSTR,
    locale: Locale,
    size: f32,
    weight: DWRITE_FONT_WEIGHT,
    alignment: DWRITE_TEXT_ALIGNMENT,
    trim: bool,
) -> Result<IDWriteTextFormat> {
    unsafe {
        let format = factory.CreateTextFormat(
            family,
            None::<&IDWriteFontCollection>,
            weight,
            DWRITE_FONT_STYLE_NORMAL,
            DWRITE_FONT_STRETCH_NORMAL,
            size,
            locale_name(locale),
        )?;
        format.SetTextAlignment(alignment)?;
        format.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
        format.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
        if trim {
            let trimming = DWRITE_TRIMMING {
                granularity: DWRITE_TRIMMING_GRANULARITY_CHARACTER,
                delimiter: 0,
                delimiterCount: 0,
            };
            let sign = factory.CreateEllipsisTrimmingSign(&format)?;
            format.SetTrimming(&trimming, &sign)?;
        }
        Ok(format)
    }
}

fn select_font_family(factory: &IDWriteFactory) -> PCWSTR {
    let mut collection = None;
    if unsafe { factory.GetSystemFontCollection(&mut collection, false) }.is_ok()
        && let Some(collection) = collection
    {
        for candidate in [w!("Segoe UI Variable Text"), w!("Segoe UI")] {
            let mut index = 0;
            let mut exists = BOOL::default();
            if unsafe { collection.FindFamilyName(candidate, &mut index, &mut exists) }.is_ok()
                && exists.as_bool()
            {
                return candidate;
            }
        }
    }
    w!("Segoe UI")
}

const fn locale_name(locale: Locale) -> PCWSTR {
    match locale {
        Locale::Chinese => w!("zh-CN"),
        Locale::English => w!("en-US"),
    }
}

struct Brushes {
    surface_alt: ID2D1SolidColorBrush,
    text: ID2D1SolidColorBrush,
    muted: ID2D1SolidColorBrush,
    line: ID2D1SolidColorBrush,
    track: ID2D1SolidColorBrush,
    accent: ID2D1SolidColorBrush,
    accent_soft: ID2D1SolidColorBrush,
    warning: ID2D1SolidColorBrush,
    footer_status: ID2D1SolidColorBrush,
    footer_text: ID2D1SolidColorBrush,
}

impl Brushes {
    fn new(
        target: &ID2D1HwndRenderTarget,
        state: &DisplayState,
        theme: Theme,
        decorations: CardDecorations,
    ) -> Result<Self> {
        let accent = accent_for_theme(state, theme);
        let accent_soft = mix_color(theme.surface, accent, if theme.dark { 0.16 } else { 0.075 });
        let track = mix_color(theme.line, theme.surface, if theme.dark { 0.20 } else { 0.32 });
        let warning_color = if theme.high_contrast {
            theme.text
        } else if theme.dark {
            rgb(244, 188, 77)
        } else {
            rgb(181, 107, 0)
        };
        let footer_status = footer_color(state, theme, decorations, accent);
        let footer_text = if state.error.is_some() { footer_status } else { theme.muted };
        Ok(Self {
            surface_alt: solid_brush(target, theme.surface_alt)?,
            text: solid_brush(target, theme.text)?,
            muted: solid_brush(target, theme.muted)?,
            line: solid_brush(target, theme.line)?,
            track: solid_brush(target, track)?,
            accent: solid_brush(target, accent)?,
            accent_soft: solid_brush(target, accent_soft)?,
            warning: solid_brush(target, warning_color)?,
            footer_status: solid_brush(target, footer_status)?,
            footer_text: solid_brush(target, footer_text)?,
        })
    }
}

fn draw_frame(
    target: &ID2D1HwndRenderTarget,
    dwrite: &IDWriteFactory,
    formats: &FormatSet,
    state: &DisplayState,
    locale: Locale,
    theme: Theme,
    decorations: CardDecorations,
) -> Result<()> {
    let brushes = Brushes::new(target, state, theme, decorations)?;
    let accent_color = accent_for_theme(state, theme);
    let accent_soft = mix_color(theme.surface, accent_color, if theme.dark { 0.16 } else { 0.075 });
    let hero_gradient = linear_gradient(
        target,
        accent_soft,
        theme.surface,
        Vector2 { X: 14.0, Y: 48.0 },
        Vector2 { X: 362.0, Y: 128.0 },
    )?;

    unsafe {
        target.Clear(Some(&color(theme.background)));
        draw_header(target, formats, &brushes, state, decorations);

        let hero = rounded_rect(14.0, 47.0, 362.0, 179.0, 14.0);
        target.FillRoundedRectangle(&hero, &hero_gradient);
        target.DrawRoundedRectangle(&hero, &brushes.line, 1.0, None::<&ID2D1StrokeStyle>);

        draw_text(
            target,
            locale.text("Weekly remaining", "本周剩余"),
            rect(28.0, 57.0, 182.0, 80.0),
            &formats.label,
            &brushes.muted,
        );
    }
    draw_percentage(target, dwrite, formats, &brushes, state.weekly_percent())?;

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

    unsafe {
        target.DrawLine(
            Vector2 { X: 203.5, Y: 64.0 },
            Vector2 { X: 203.5, Y: 130.0 },
            &brushes.line,
            1.0,
            None::<&ID2D1StrokeStyle>,
        );
        draw_text(
            target,
            locale.text("Reset in", "距离重置"),
            rect(219.0, 57.0, 346.0, 80.0),
            &formats.label,
            &brushes.muted,
        );
        draw_text(target, &reset.0, rect(219.0, 78.0, 347.0, 108.0), &formats.value, &brushes.text);
        draw_text(
            target,
            &reset.1,
            rect(219.0, 106.0, 347.0, 130.0),
            &formats.secondary,
            &brushes.muted,
        );
    }

    draw_quota_track(target, formats, &brushes, state, locale, theme, accent_color)?;
    draw_metrics(target, formats, &brushes, state, locale);
    draw_footer(target, formats, &brushes, state, locale, decorations);
    Ok(())
}

unsafe fn draw_header(
    target: &ID2D1HwndRenderTarget,
    formats: &FormatSet,
    brushes: &Brushes,
    state: &DisplayState,
    decorations: CardDecorations,
) {
    unsafe {
        target.FillEllipse(&ellipse(22.0, 23.0, 3.2), &brushes.accent);
        draw_text(
            target,
            "CodexStatus",
            rect(32.0, 7.0, 184.0, 40.0),
            &formats.header,
            &brushes.text,
        );
        draw_text(
            target,
            &updated_text(state, formats.locale),
            rect(186.0, 8.0, 330.0, 39.0),
            &formats.update,
            &brushes.muted,
        );

        let button = rounded_rect(337.0, 10.0, 361.0, 35.0, 7.0);
        target.FillRoundedRectangle(
            &button,
            if decorations.pinned { &brushes.accent_soft } else { &brushes.surface_alt },
        );
        if decorations.pinned {
            target.DrawRoundedRectangle(&button, &brushes.accent, 1.0, None::<&ID2D1StrokeStyle>);
        }
        let pin_brush = if decorations.pinned { &brushes.accent } else { &brushes.muted };
        target.FillRoundedRectangle(&rounded_rect(344.5, 15.5, 353.5, 19.5, 2.0), pin_brush);
        target.DrawLine(
            Vector2 { X: 349.0, Y: 19.0 },
            Vector2 { X: 349.0, Y: 29.0 },
            pin_brush,
            1.4,
            None::<&ID2D1StrokeStyle>,
        );
        target.DrawLine(
            Vector2 { X: 345.5, Y: 23.0 },
            Vector2 { X: 352.5, Y: 23.0 },
            pin_brush,
            1.4,
            None::<&ID2D1StrokeStyle>,
        );
    }
}

fn draw_percentage(
    target: &ID2D1HwndRenderTarget,
    dwrite: &IDWriteFactory,
    formats: &FormatSet,
    brushes: &Brushes,
    percent: Option<u8>,
) -> Result<()> {
    let number = percent.map_or_else(|| "--".to_owned(), |value| value.to_string());
    let width = text_width(dwrite, &number, &formats.quota, 160.0, 58.0)?;
    unsafe {
        draw_text(target, &number, rect(28.0, 76.0, 186.0, 134.0), &formats.quota, &brushes.text);
        if percent.is_some() {
            draw_text(
                target,
                "%",
                rect(31.0 + width, 89.0, 188.0, 130.0),
                &formats.percent,
                &brushes.muted,
            );
        }
    }
    Ok(())
}

fn draw_quota_track(
    target: &ID2D1HwndRenderTarget,
    formats: &FormatSet,
    brushes: &Brushes,
    state: &DisplayState,
    locale: Locale,
    theme: Theme,
    accent_color: COLORREF,
) -> Result<()> {
    let left = 28.0;
    let right = 348.0;
    let top = 143.0;
    let bottom = 149.0;
    unsafe {
        target.FillRoundedRectangle(&rounded_rect(left, top, right, bottom, 3.0), &brushes.track)
    };

    if let Some(value) = state.weekly_percent()
        && value > 0
    {
        let filled = (left + (right - left) * f32::from(value) / 100.0).clamp(left + 6.0, right);
        let start_color =
            mix_color(accent_color, theme.surface, if theme.dark { 0.08 } else { 0.16 });
        let progress = linear_gradient(
            target,
            start_color,
            accent_color,
            Vector2 { X: left, Y: top },
            Vector2 { X: right, Y: top },
        )?;
        unsafe {
            target.FillRoundedRectangle(&rounded_rect(left, top, filled, bottom, 3.0), &progress);
            if value < 100 {
                target.FillEllipse(&ellipse(filled, (top + bottom) / 2.0, 2.5), &brushes.accent);
            }
        }
    }

    if let Some(window) = state.snapshot.as_ref().and_then(|snapshot| snapshot.weekly.as_ref()) {
        let insight = analyze_window(window, Local::now().timestamp());
        if let Some(elapsed) = insight.elapsed_percent {
            let expected_remaining = (100.0 - elapsed).clamp(0.0, 100.0) as f32;
            let marker = left + (right - left) * expected_remaining / 100.0;
            unsafe {
                target.DrawLine(
                    Vector2 { X: marker, Y: top - 3.0 },
                    Vector2 { X: marker, Y: bottom + 3.0 },
                    &brushes.muted,
                    1.0,
                    None::<&ID2D1StrokeStyle>,
                );
            }
            let pace_warning = insight.is_ahead_of_pace && insight.likely_exhaust_before_reset;
            unsafe {
                draw_text(
                    target,
                    if pace_warning {
                        locale
                            .text("Usage pace high · may run out early", "用量偏快 · 可能提前耗尽")
                    } else {
                        locale.text("Usage pace on track", "用量节奏正常")
                    },
                    rect(28.0, 151.0, 348.0, 174.0),
                    &formats.secondary,
                    if pace_warning { &brushes.warning } else { &brushes.muted },
                );
            }
        }
    }
    Ok(())
}

fn draw_metrics(
    target: &ID2D1HwndRenderTarget,
    formats: &FormatSet,
    brushes: &Brushes,
    state: &DisplayState,
    locale: Locale,
) {
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

    unsafe {
        target.DrawLine(
            Vector2 { X: 18.0, Y: 188.5 },
            Vector2 { X: 358.0, Y: 188.5 },
            &brushes.line,
            1.0,
            None::<&ID2D1StrokeStyle>,
        );
    }

    if let Some(session) = session {
        draw_metric_divider(target, brushes, 130.5);
        draw_metric_divider(target, brushes, 246.5);
        draw_metric(
            target,
            formats,
            brushes,
            rect(17.0, 190.0, 130.0, 252.0),
            locale.text("5-hour quota", "5 小时额度"),
            &session,
        );
        draw_metric(
            target,
            formats,
            brushes,
            rect(131.0, 190.0, 246.0, 252.0),
            locale.text("Plan", "套餐"),
            &plan,
        );
        draw_metric(
            target,
            formats,
            brushes,
            rect(247.0, 190.0, 359.0, 252.0),
            locale.text("Reset credits", "重置机会"),
            &credits,
        );
    } else {
        draw_metric_divider(target, brushes, 188.5);
        draw_metric(
            target,
            formats,
            brushes,
            rect(17.0, 190.0, 188.0, 252.0),
            locale.text("Plan", "套餐"),
            &plan,
        );
        draw_metric(
            target,
            formats,
            brushes,
            rect(189.0, 190.0, 359.0, 252.0),
            locale.text("Reset credits", "重置机会"),
            &credits,
        );
    }
}

fn draw_metric_divider(target: &ID2D1HwndRenderTarget, brushes: &Brushes, x: f32) {
    unsafe {
        target.DrawLine(
            Vector2 { X: x, Y: 201.0 },
            Vector2 { X: x, Y: 244.0 },
            &brushes.line,
            1.0,
            None::<&ID2D1StrokeStyle>,
        );
    }
}

fn draw_metric(
    target: &ID2D1HwndRenderTarget,
    formats: &FormatSet,
    brushes: &Brushes,
    area: D2D_RECT_F,
    label: &str,
    value: &str,
) {
    unsafe {
        draw_text(
            target,
            label,
            rect(area.left + 11.0, area.top + 3.0, area.right - 9.0, area.top + 27.0),
            &formats.metric_label,
            &brushes.muted,
        );
        draw_text(
            target,
            value,
            rect(area.left + 11.0, area.top + 25.0, area.right - 9.0, area.bottom - 3.0),
            &formats.metric_value,
            &brushes.text,
        );
    }
}

fn draw_footer(
    target: &ID2D1HwndRenderTarget,
    formats: &FormatSet,
    brushes: &Brushes,
    state: &DisplayState,
    locale: Locale,
    decorations: CardDecorations,
) {
    unsafe {
        target.DrawLine(
            Vector2 { X: 18.0, Y: 258.5 },
            Vector2 { X: 358.0, Y: 258.5 },
            &brushes.line,
            1.0,
            None::<&ID2D1StrokeStyle>,
        );
        target.FillEllipse(&ellipse(21.5, 276.0, 2.5), &brushes.footer_status);
        draw_text(
            target,
            &footer_text(state, locale, decorations.service_health),
            rect(29.0, 262.0, 358.0, 290.0),
            &formats.footer,
            &brushes.footer_text,
        );
    }
}

unsafe fn draw_text(
    target: &ID2D1HwndRenderTarget,
    value: &str,
    area: D2D_RECT_F,
    format: &IDWriteTextFormat,
    brush: &ID2D1SolidColorBrush,
) {
    let text: Vec<u16> = value.encode_utf16().collect();
    unsafe {
        target.DrawText(
            &text,
            format,
            &area,
            brush,
            D2D1_DRAW_TEXT_OPTIONS_NONE,
            DWRITE_MEASURING_MODE_NATURAL,
        )
    };
}

fn text_width(
    factory: &IDWriteFactory,
    value: &str,
    format: &IDWriteTextFormat,
    max_width: f32,
    max_height: f32,
) -> Result<f32> {
    let text: Vec<u16> = value.encode_utf16().collect();
    let layout = unsafe { factory.CreateTextLayout(&text, format, max_width, max_height)? };
    let mut metrics = DWRITE_TEXT_METRICS::default();
    unsafe { layout.GetMetrics(&mut metrics)? };
    Ok(metrics.widthIncludingTrailingWhitespace)
}

fn solid_brush(target: &ID2D1HwndRenderTarget, value: COLORREF) -> Result<ID2D1SolidColorBrush> {
    unsafe { target.CreateSolidColorBrush(&color(value), None) }
}

fn linear_gradient(
    target: &ID2D1HwndRenderTarget,
    start_color: COLORREF,
    end_color: COLORREF,
    start: Vector2,
    end: Vector2,
) -> Result<ID2D1LinearGradientBrush> {
    let stops = [
        D2D1_GRADIENT_STOP { position: 0.0, color: color(start_color) },
        D2D1_GRADIENT_STOP { position: 1.0, color: color(end_color) },
    ];
    unsafe {
        let collection =
            target.CreateGradientStopCollection(&stops, D2D1_GAMMA_2_2, D2D1_EXTEND_MODE_CLAMP)?;
        let properties = D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES { startPoint: start, endPoint: end };
        target.CreateLinearGradientBrush(&properties, None, &collection)
    }
}

fn accent_for_theme(state: &DisplayState, theme: Theme) -> COLORREF {
    if theme.high_contrast || !theme.dark {
        return accent_for(state, theme.high_contrast);
    }
    if state.refresh_state != RefreshState::Live {
        return rgb(137, 161, 182);
    }
    match state.weekly_percent() {
        Some(value) if value < 20 => rgb(255, 119, 132),
        Some(value) if value < 50 => rgb(244, 188, 77),
        Some(_) => rgb(49, 205, 160),
        None => rgb(154, 164, 168),
    }
}

fn footer_color(
    state: &DisplayState,
    theme: Theme,
    decorations: CardDecorations,
    accent: COLORREF,
) -> COLORREF {
    if state.error.is_some() {
        return accent;
    }
    if theme.high_contrast {
        return theme.text;
    }
    match decorations.service_health {
        ServiceHealth::Degraded => {
            if theme.dark {
                rgb(244, 188, 77)
            } else {
                rgb(181, 107, 0)
            }
        }
        ServiceHealth::Outage => {
            if theme.dark {
                rgb(255, 119, 132)
            } else {
                rgb(202, 55, 70)
            }
        }
        ServiceHealth::Operational => accent,
        ServiceHealth::Unknown => theme.muted,
    }
}

fn color(value: COLORREF) -> D2D1_COLOR_F {
    D2D1_COLOR_F {
        r: (value.0 & 0xff) as f32 / 255.0,
        g: ((value.0 >> 8) & 0xff) as f32 / 255.0,
        b: ((value.0 >> 16) & 0xff) as f32 / 255.0,
        a: 1.0,
    }
}

fn mix_color(from: COLORREF, to: COLORREF, amount: f32) -> COLORREF {
    let amount = amount.clamp(0.0, 1.0);
    let mix = |shift: u32| {
        let from = ((from.0 >> shift) & 0xff) as f32;
        let to = ((to.0 >> shift) & 0xff) as f32;
        (from + (to - from) * amount).round().clamp(0.0, 255.0) as u8
    };
    rgb(mix(0), mix(8), mix(16))
}

const fn rect(left: f32, top: f32, right: f32, bottom: f32) -> D2D_RECT_F {
    D2D_RECT_F { left, top, right, bottom }
}

const fn rounded_rect(
    left: f32,
    top: f32,
    right: f32,
    bottom: f32,
    radius: f32,
) -> D2D1_ROUNDED_RECT {
    D2D1_ROUNDED_RECT { rect: rect(left, top, right, bottom), radiusX: radius, radiusY: radius }
}

const fn ellipse(x: f32, y: f32, radius: f32) -> windows::Win32::Graphics::Direct2D::D2D1_ELLIPSE {
    windows::Win32::Graphics::Direct2D::D2D1_ELLIPSE {
        point: Vector2 { X: x, Y: y },
        radiusX: radius,
        radiusY: radius,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn color_mixing_keeps_channel_order() {
        assert_eq!(mix_color(rgb(0, 0, 0), rgb(100, 150, 200), 0.5), rgb(50, 75, 100));
    }

    #[test]
    fn card_geometry_stays_inside_the_logical_surface() {
        let hero = rounded_rect(14.0, 47.0, 362.0, 179.0, 14.0);
        assert!(hero.rect.left >= 0.0);
        assert!(hero.rect.right <= super::super::CARD_WIDTH as f32);
        assert!(hero.rect.bottom <= super::super::CARD_HEIGHT as f32);
    }
}
