//! Direct2D/DirectWrite renderer for the CodexStatus flyout.
//!
//! THESIS: the quota is a calm, luminous instrument, not a flat information
//! table. OWN WORLD: pearl or frosted-graphite canvas, elevated soft panels,
//! emerald light, generous 18-DIP radii, restrained shadows, and one Segoe UI
//! Variable type system. STORY: weekly quota first, then reset timing and pace,
//! followed by plan, optional five-hour quota, and reset credits. FIRST VIEWPORT:
//! 420 × 430 DIPs; a compact header, one hero panel, one supporting panel, and a
//! quiet privacy footer. FORM: the user-pinned Stitch “Compact Glass” reference;
//! no randomized composition was used.

use super::{
    Locale, Theme, UsageProjection, footer_text, plan_label, projection_label, reset_details, rgb,
    updated_text, weekly_usage_projection,
};
use crate::insights::analyze_window;
use crate::model::DisplayState;
use chrono::Local;
use std::cell::RefCell;
use std::mem::size_of;
use windows::Win32::Foundation::{COLORREF, D2DERR_RECREATE_TARGET, HMODULE, HWND};
use windows::Win32::Graphics::Direct2D::Common::{
    D2D_RECT_F, D2D_SIZE_U, D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_COLOR_F,
    D2D1_COMPOSITE_MODE_SOURCE_OVER, D2D1_FIGURE_BEGIN_FILLED, D2D1_FIGURE_END_CLOSED,
    D2D1_GRADIENT_STOP, D2D1_PIXEL_FORMAT,
};
use windows::Win32::Graphics::Direct2D::{
    CLSID_D2D1GaussianBlur, CLSID_D2D1Shadow, D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
    D2D1_BITMAP_OPTIONS_CANNOT_DRAW, D2D1_BITMAP_OPTIONS_TARGET, D2D1_BITMAP_PROPERTIES1,
    D2D1_BUFFER_PRECISION_8BPC_UNORM, D2D1_COLOR_INTERPOLATION_MODE_STRAIGHT,
    D2D1_COLOR_SPACE_SRGB, D2D1_DEVICE_CONTEXT_OPTIONS_NONE, D2D1_DRAW_TEXT_OPTIONS_NONE,
    D2D1_EXTEND_MODE_CLAMP, D2D1_FACTORY_TYPE_SINGLE_THREADED,
    D2D1_GAUSSIANBLUR_PROP_STANDARD_DEVIATION, D2D1_INTERPOLATION_MODE_LINEAR,
    D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES, D2D1_PROPERTY_TYPE_FLOAT, D2D1_PROPERTY_TYPE_VECTOR4,
    D2D1_ROUNDED_RECT, D2D1_SHADOW_PROP_BLUR_STANDARD_DEVIATION, D2D1_SHADOW_PROP_COLOR,
    D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE, D2D1CreateFactory, ID2D1Bitmap1, ID2D1Brush,
    ID2D1CommandList, ID2D1Device, ID2D1DeviceContext, ID2D1Factory1, ID2D1Image,
    ID2D1LinearGradientBrush, ID2D1SolidColorBrush, ID2D1StrokeStyle,
};
use windows::Win32::Graphics::Direct3D::{
    D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_WARP, D3D_FEATURE_LEVEL_11_0,
};
use windows::Win32::Graphics::Direct3D11::{
    D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_SDK_VERSION, D3D11CreateDevice, ID3D11Device,
};
use windows::Win32::Graphics::DirectComposition::{
    DCompositionCreateDevice, IDCompositionDevice, IDCompositionTarget, IDCompositionVisual,
};
use windows::Win32::Graphics::DirectWrite::{
    DWRITE_FACTORY_TYPE_SHARED, DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE_NORMAL,
    DWRITE_FONT_WEIGHT, DWRITE_FONT_WEIGHT_NORMAL, DWRITE_FONT_WEIGHT_SEMI_BOLD,
    DWRITE_MEASURING_MODE_NATURAL, DWRITE_PARAGRAPH_ALIGNMENT_CENTER, DWRITE_TEXT_ALIGNMENT,
    DWRITE_TEXT_ALIGNMENT_CENTER, DWRITE_TEXT_ALIGNMENT_LEADING, DWRITE_TEXT_ALIGNMENT_TRAILING,
    DWRITE_TEXT_METRICS, DWRITE_TRIMMING, DWRITE_TRIMMING_GRANULARITY_CHARACTER,
    DWRITE_WORD_WRAPPING_NO_WRAP, DWriteCreateFactory, IDWriteFactory, IDWriteFontCollection,
    IDWriteTextFormat,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_ALPHA_MODE_PREMULTIPLIED, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_FORMAT_UNKNOWN,
    DXGI_SAMPLE_DESC,
};
use windows::Win32::Graphics::Dxgi::{
    DXGI_ERROR_DEVICE_HUNG, DXGI_ERROR_DEVICE_REMOVED, DXGI_ERROR_DEVICE_RESET, DXGI_PRESENT,
    DXGI_SCALING_STRETCH, DXGI_SWAP_CHAIN_DESC1, DXGI_SWAP_CHAIN_FLAG,
    DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL, DXGI_USAGE_RENDER_TARGET_OUTPUT, IDXGIAdapter, IDXGIDevice,
    IDXGIFactory2, IDXGISurface, IDXGISwapChain1,
};
use windows::core::{BOOL, Interface, PCWSTR, Result, w};
use windows_numerics::Vector2;

thread_local! {
    static RENDERER: RefCell<Option<Renderer>> = const { RefCell::new(None) };
    static GDI_FALLBACK_ACTIVE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static FOLLOWUP_PAINT_REQUESTED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static GDI_FRAME_REQUIRES_OPAQUE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static DEVICE_LOST_RETRIES: std::cell::Cell<u8> = const { std::cell::Cell::new(0) };
}

const MAX_DEVICE_LOST_RETRIES: u8 = 1;

pub(super) struct PaintInput<'a> {
    pub hwnd: HWND,
    pub size: (i32, i32),
    pub dpi: u32,
    pub state: &'a DisplayState,
    pub locale: Locale,
    pub theme: Theme,
    pub glass_enabled: bool,
}

pub(super) fn paint(input: PaintInput<'_>) -> bool {
    if GDI_FALLBACK_ACTIVE.with(std::cell::Cell::get) {
        return false;
    }
    RENDERER.with(|slot| {
        let mut slot = slot.borrow_mut();
        if slot.is_none() {
            match Renderer::new() {
                Ok(renderer) => *slot = Some(renderer),
                Err(error) => {
                    diagnostic_failure("initialize", &error);
                    activate_gdi_fallback(input.hwnd);
                    return false;
                }
            }
        }

        let result = slot.as_mut().expect("renderer initialized").paint(&input);
        if let Err(error) = result {
            diagnostic_failure("paint", &error);
            let device_lost = is_device_lost(error.code());
            let detach_failed =
                slot.as_mut().is_some_and(|renderer| renderer.detach_surface().is_err());
            if device_lost {
                // D2DERR_RECREATE_TARGET and DXGI device-removal errors invalidate
                // the complete D3D/D2D/DComp tree, not just the back buffer.
                *slot = None;
                let attempts = DEVICE_LOST_RETRIES.with(|retries| {
                    let attempts = retries.get();
                    retries.set(attempts.saturating_add(1));
                    attempts
                });
                if attempts < MAX_DEVICE_LOST_RETRIES {
                    activate_temporary_gdi_frame(input.hwnd);
                } else {
                    activate_gdi_fallback(input.hwnd);
                }
            } else {
                // If DComp could not commit the detached root, discard every
                // device/target reference before GDI takes over.
                if detach_failed {
                    *slot = None;
                }
                activate_gdi_fallback(input.hwnd);
            }
            return false;
        }
        DEVICE_LOST_RETRIES.with(|retries| retries.set(0));
        true
    })
}

pub(super) fn release_surface() {
    RENDERER.with(|slot| {
        let detach_failed =
            slot.borrow_mut().as_mut().is_some_and(|renderer| renderer.detach_surface().is_err());
        if detach_failed {
            *slot.borrow_mut() = None;
        }
    });
    reset_after_hidden();
}

pub(super) fn release_device_tree() {
    RENDERER.with(|slot| {
        if let Some(mut renderer) = slot.borrow_mut().take() {
            // Keep the expensive D3D11/D2D device tree warm. Only HWND-sized
            // composition resources are released on hide; this path is called
            // only after the idle grace period expires.
            let _ = renderer.detach_surface();
        }
    });
    reset_after_hidden();
}

fn reset_after_hidden() {
    GDI_FALLBACK_ACTIVE.with(|active| active.set(false));
    FOLLOWUP_PAINT_REQUESTED.with(|pending| pending.set(false));
    GDI_FRAME_REQUIRES_OPAQUE.with(|required| required.set(false));
    DEVICE_LOST_RETRIES.with(|retries| retries.set(0));
}

pub(super) fn take_followup_paint_request() -> bool {
    FOLLOWUP_PAINT_REQUESTED.with(|pending| pending.replace(false))
}

pub(super) fn gdi_fallback_active() -> bool {
    GDI_FALLBACK_ACTIVE.with(std::cell::Cell::get)
}

pub(super) fn take_gdi_frame_requires_opaque() -> bool {
    GDI_FRAME_REQUIRES_OPAQUE.with(|required| required.replace(false))
}

fn activate_gdi_fallback(_hwnd: HWND) {
    GDI_FALLBACK_ACTIVE.with(|active| active.set(true));
    GDI_FRAME_REQUIRES_OPAQUE.with(|required| required.set(true));
    // Some systems do not expose a freshly detached/reconfigured HWND to GDI
    // in the same WM_PAINT. The immediate paint still attempts GDI, while this
    // one-shot post-EndPaint invalidation guarantees a clean GDI-only frame.
    FOLLOWUP_PAINT_REQUESTED.with(|pending| pending.set(true));
}

fn activate_temporary_gdi_frame(_hwnd: HWND) {
    GDI_FRAME_REQUIRES_OPAQUE.with(|required| required.set(true));
    FOLLOWUP_PAINT_REQUESTED.with(|pending| pending.set(true));
}

#[cfg(feature = "diagnostics")]
fn diagnostic_failure(stage: &str, error: &windows::core::Error) {
    eprintln!("Direct2D {stage} failed: {:#x}", error.code().0);
}

#[cfg(not(feature = "diagnostics"))]
fn diagnostic_failure(_stage: &str, _error: &windows::core::Error) {}

struct Renderer {
    d3d_device: ID3D11Device,
    dxgi_device: IDXGIDevice,
    _d2d_factory: ID2D1Factory1,
    _d2d_device: ID2D1Device,
    context: ID2D1DeviceContext,
    dwrite: IDWriteFactory,
    font_family: PCWSTR,
    surface: Option<CompositionSurface>,
    formats: Option<FormatSet>,
}

impl Renderer {
    fn new() -> Result<Self> {
        unsafe {
            let d3d_device = create_d3d_device()?;
            let dxgi_device: IDXGIDevice = d3d_device.cast()?;
            let d2d_factory =
                D2D1CreateFactory::<ID2D1Factory1>(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
            let d2d_device = d2d_factory.CreateDevice(&dxgi_device)?;
            let context = d2d_device.CreateDeviceContext(D2D1_DEVICE_CONTEXT_OPTIONS_NONE)?;
            context.SetAntialiasMode(D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
            let dwrite = DWriteCreateFactory::<IDWriteFactory>(DWRITE_FACTORY_TYPE_SHARED)?;
            let font_family = select_font_family(&dwrite);
            Ok(Self {
                d3d_device,
                dxgi_device,
                _d2d_factory: d2d_factory,
                _d2d_device: d2d_device,
                context,
                dwrite,
                font_family,
                surface: None,
                formats: None,
            })
        }
    }

    fn paint(&mut self, input: &PaintInput<'_>) -> Result<()> {
        self.ensure_surface(input.hwnd, input.size.0, input.size.1, input.dpi)?;
        self.ensure_formats(input.locale)?;

        let context = self.context.clone();
        let formats = self.formats.as_ref().expect("formats initialized").clone();
        let dwrite = self.dwrite.clone();
        unsafe {
            // The composition target is premultiplied in every material mode.
            // Direct2D cannot provide real ClearType unless alpha is IGNORE.
            context.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
            context.BeginDraw();
        }
        let draw_result = draw_frame(
            &context,
            &dwrite,
            &formats,
            input.state,
            input.locale,
            input.theme,
            input.glass_enabled,
        );
        let end_result = unsafe { context.EndDraw(None, None) };
        end_result?;
        draw_result?;

        let surface = self.surface.as_ref().expect("surface initialized");
        unsafe {
            surface.swap_chain.Present(1, DXGI_PRESENT(0)).ok()?;
            surface.dcomp_device.Commit()?;
        }
        Ok(())
    }

    fn ensure_surface(&mut self, hwnd: HWND, width: i32, height: i32, dpi: u32) -> Result<()> {
        let size = D2D_SIZE_U { width: width.max(1) as u32, height: height.max(1) as u32 };
        let wrong_window = self.surface.as_ref().is_some_and(|surface| surface.hwnd != hwnd);
        if wrong_window {
            self.detach_surface()?;
        }

        if let Some(surface) = &mut self.surface {
            surface.resize(size, dpi, &self.context)?;
        } else {
            self.surface = Some(CompositionSurface::new(
                hwnd,
                size,
                dpi,
                &self.d3d_device,
                &self.dxgi_device,
                &self.context,
            )?);
        }
        Ok(())
    }

    fn ensure_formats(&mut self, locale: Locale) -> Result<()> {
        if self.formats.as_ref().is_none_or(|formats| formats.locale != locale) {
            self.formats = Some(FormatSet::new(&self.dwrite, self.font_family, locale)?);
        }
        Ok(())
    }

    fn detach_surface(&mut self) -> Result<()> {
        unsafe {
            self.context.SetTarget(None::<&ID2D1Image>);
        }
        if let Some(surface) = self.surface.take() {
            surface.detach()?;
        }
        Ok(())
    }
}

struct CompositionSurface {
    hwnd: HWND,
    swap_chain: IDXGISwapChain1,
    dcomp_device: IDCompositionDevice,
    dcomp_target: IDCompositionTarget,
    _visual: IDCompositionVisual,
    target_bitmap: Option<ID2D1Bitmap1>,
    pixel_size: D2D_SIZE_U,
    dpi: u32,
}

impl CompositionSurface {
    fn new(
        hwnd: HWND,
        pixel_size: D2D_SIZE_U,
        dpi: u32,
        d3d_device: &ID3D11Device,
        dxgi_device: &IDXGIDevice,
        context: &ID2D1DeviceContext,
    ) -> Result<Self> {
        unsafe {
            let adapter: IDXGIAdapter = dxgi_device.GetAdapter()?;
            let factory: IDXGIFactory2 = adapter.GetParent()?;
            let desc = DXGI_SWAP_CHAIN_DESC1 {
                Width: pixel_size.width,
                Height: pixel_size.height,
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                Stereo: false.into(),
                SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
                BufferCount: 2,
                Scaling: DXGI_SCALING_STRETCH,
                SwapEffect: DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL,
                AlphaMode: DXGI_ALPHA_MODE_PREMULTIPLIED,
                Flags: 0,
            };
            let swap_chain = factory.CreateSwapChainForComposition(d3d_device, &desc, None)?;
            let dcomp_device: IDCompositionDevice = DCompositionCreateDevice(dxgi_device)?;
            let dcomp_target = dcomp_device.CreateTargetForHwnd(hwnd, true)?;
            let visual = dcomp_device.CreateVisual()?;
            let target_bitmap = create_target_bitmap(context, &swap_chain, dpi)?;

            // Attach only after every back-buffer-dependent object exists. If
            // an earlier step fails, no transparent composition root can cover
            // the HWND and the caller can immediately use GDI.
            visual.SetContent(&swap_chain)?;
            dcomp_target.SetRoot(&visual)?;
            if let Err(error) = dcomp_device.Commit() {
                let detach_result = dcomp_target
                    .SetRoot(None::<&IDCompositionVisual>)
                    .and_then(|()| dcomp_device.Commit());
                if let Err(detach_error) = detach_result {
                    // Returning drops the uncommitted device, target, visual,
                    // and swapchain together. Record the secondary failure
                    // instead of silently pretending the root was removed.
                    diagnostic_failure("initial composition detach", &detach_error);
                }
                return Err(error);
            }
            context.SetTarget(&target_bitmap);
            context.SetDpi(dpi as f32, dpi as f32);

            Ok(Self {
                hwnd,
                swap_chain,
                dcomp_device,
                dcomp_target,
                _visual: visual,
                target_bitmap: Some(target_bitmap),
                pixel_size,
                dpi,
            })
        }
    }

    fn resize(
        &mut self,
        pixel_size: D2D_SIZE_U,
        dpi: u32,
        context: &ID2D1DeviceContext,
    ) -> Result<()> {
        if pixel_size == self.pixel_size && dpi == self.dpi {
            return Ok(());
        }
        unsafe {
            // DXGI requires every reference to the current back buffer to be
            // released before ResizeBuffers.
            context.SetTarget(None::<&ID2D1Image>);
            self.target_bitmap.take();
            if pixel_size != self.pixel_size {
                self.swap_chain.ResizeBuffers(
                    0,
                    pixel_size.width,
                    pixel_size.height,
                    DXGI_FORMAT_UNKNOWN,
                    DXGI_SWAP_CHAIN_FLAG(0),
                )?;
            }
            let target_bitmap = create_target_bitmap(context, &self.swap_chain, dpi)?;
            context.SetTarget(&target_bitmap);
            context.SetDpi(dpi as f32, dpi as f32);
            self.target_bitmap = Some(target_bitmap);
            self.pixel_size = pixel_size;
            self.dpi = dpi;
        }
        Ok(())
    }

    fn detach(self) -> Result<()> {
        unsafe {
            // Detach and commit before dropping the swapchain so GDI can own
            // the HWND; ui.rs schedules a follow-up frame when the first
            // WM_PAINT cannot expose the redirected GDI surface yet.
            self.dcomp_target.SetRoot(None::<&IDCompositionVisual>)?;
            self.dcomp_device.Commit()?;
        }
        Ok(())
    }
}

fn create_d3d_device() -> Result<ID3D11Device> {
    let mut last_error = None;
    for driver in [D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_WARP] {
        let mut device = None;
        let result = unsafe {
            D3D11CreateDevice(
                None,
                driver,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                Some(&[D3D_FEATURE_LEVEL_11_0]),
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                None,
            )
        };
        match result {
            Ok(()) => {
                if let Some(device) = device {
                    return Ok(device);
                }
            }
            Err(error) => last_error = Some(error),
        }
    }
    Err(last_error.unwrap_or_else(windows::core::Error::from_thread))
}

fn create_target_bitmap(
    context: &ID2D1DeviceContext,
    swap_chain: &IDXGISwapChain1,
    dpi: u32,
) -> Result<ID2D1Bitmap1> {
    let surface: IDXGISurface = unsafe { swap_chain.GetBuffer(0)? };
    let properties = D2D1_BITMAP_PROPERTIES1 {
        pixelFormat: D2D1_PIXEL_FORMAT {
            format: DXGI_FORMAT_B8G8R8A8_UNORM,
            alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
        },
        dpiX: dpi as f32,
        dpiY: dpi as f32,
        bitmapOptions: D2D1_BITMAP_OPTIONS_TARGET | D2D1_BITMAP_OPTIONS_CANNOT_DRAW,
        ..Default::default()
    };
    unsafe { context.CreateBitmapFromDxgiSurface(&surface, Some(&properties)) }
}

fn is_device_lost(code: windows::core::HRESULT) -> bool {
    code == D2DERR_RECREATE_TARGET
        || code == DXGI_ERROR_DEVICE_REMOVED
        || code == DXGI_ERROR_DEVICE_RESET
        || code == DXGI_ERROR_DEVICE_HUNG
}

#[derive(Clone)]
struct FormatSet {
    locale: Locale,
    header: IDWriteTextFormat,
    update: IDWriteTextFormat,
    label: IDWriteTextFormat,
    quota: IDWriteTextFormat,
    percent: IDWriteTextFormat,
    reset_value: IDWriteTextFormat,
    secondary: IDWriteTextFormat,
    pace: IDWriteTextFormat,
    metric_label: IDWriteTextFormat,
    metric_value: IDWriteTextFormat,
    badge: IDWriteTextFormat,
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
                18.0,
                DWRITE_FONT_WEIGHT_SEMI_BOLD,
                DWRITE_TEXT_ALIGNMENT_LEADING,
                false,
            )?,
            update: make_format(
                factory,
                family,
                locale,
                12.5,
                DWRITE_FONT_WEIGHT_NORMAL,
                DWRITE_TEXT_ALIGNMENT_TRAILING,
                true,
            )?,
            label: make_format(
                factory,
                family,
                locale,
                15.0,
                DWRITE_FONT_WEIGHT_NORMAL,
                DWRITE_TEXT_ALIGNMENT_LEADING,
                true,
            )?,
            quota: make_format(
                factory,
                family,
                locale,
                64.0,
                DWRITE_FONT_WEIGHT_SEMI_BOLD,
                DWRITE_TEXT_ALIGNMENT_LEADING,
                false,
            )?,
            percent: make_format(
                factory,
                family,
                locale,
                29.0,
                DWRITE_FONT_WEIGHT_SEMI_BOLD,
                DWRITE_TEXT_ALIGNMENT_LEADING,
                false,
            )?,
            reset_value: make_format(
                factory,
                family,
                locale,
                20.0,
                DWRITE_FONT_WEIGHT_SEMI_BOLD,
                DWRITE_TEXT_ALIGNMENT_LEADING,
                true,
            )?,
            secondary: make_format(
                factory,
                family,
                locale,
                12.5,
                DWRITE_FONT_WEIGHT_NORMAL,
                DWRITE_TEXT_ALIGNMENT_LEADING,
                true,
            )?,
            pace: make_format(
                factory,
                family,
                locale,
                14.0,
                DWRITE_FONT_WEIGHT_SEMI_BOLD,
                DWRITE_TEXT_ALIGNMENT_LEADING,
                true,
            )?,
            metric_label: make_format(
                factory,
                family,
                locale,
                14.0,
                DWRITE_FONT_WEIGHT_NORMAL,
                DWRITE_TEXT_ALIGNMENT_LEADING,
                true,
            )?,
            metric_value: make_format(
                factory,
                family,
                locale,
                27.0,
                DWRITE_FONT_WEIGHT_SEMI_BOLD,
                DWRITE_TEXT_ALIGNMENT_LEADING,
                true,
            )?,
            badge: make_format(
                factory,
                family,
                locale,
                14.0,
                DWRITE_FONT_WEIGHT_SEMI_BOLD,
                DWRITE_TEXT_ALIGNMENT_CENTER,
                true,
            )?,
            footer: make_format(
                factory,
                family,
                locale,
                12.0,
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
    text: ID2D1SolidColorBrush,
    muted: ID2D1SolidColorBrush,
    line: ID2D1SolidColorBrush,
    track: ID2D1SolidColorBrush,
    accent: ID2D1SolidColorBrush,
    accent_glow_soft: ID2D1SolidColorBrush,
    accent_glow: ID2D1SolidColorBrush,
    warning: ID2D1SolidColorBrush,
    error: ID2D1SolidColorBrush,
}

impl Brushes {
    fn new(target: &ID2D1DeviceContext, state: &DisplayState, theme: Theme) -> Result<Self> {
        let accent = accent_for_theme(state, theme);
        let track = mix_color(theme.line, theme.surface, if theme.dark { 0.12 } else { 0.28 });
        let warning = if theme.high_contrast {
            theme.text
        } else if theme.dark {
            rgb(255, 190, 86)
        } else {
            rgb(184, 111, 17)
        };
        let error = if theme.high_contrast {
            theme.text
        } else if theme.dark {
            rgb(255, 126, 137)
        } else {
            rgb(202, 55, 70)
        };
        let glow_allowed = !theme.high_contrast;
        Ok(Self {
            text: solid_brush(target, theme.text)?,
            muted: solid_brush(target, theme.muted)?,
            line: solid_brush(target, theme.line)?,
            track: solid_brush(target, track)?,
            accent: solid_brush(target, accent)?,
            accent_glow_soft: solid_brush_alpha(
                target,
                accent,
                if glow_allowed { 0.10 } else { 0.0 },
            )?,
            accent_glow: solid_brush_alpha(target, accent, if glow_allowed { 0.25 } else { 0.0 })?,
            warning: solid_brush(target, warning)?,
            error: solid_brush(target, error)?,
        })
    }
}

fn draw_frame(
    target: &ID2D1DeviceContext,
    dwrite: &IDWriteFactory,
    formats: &FormatSet,
    state: &DisplayState,
    locale: Locale,
    theme: Theme,
    glass_enabled: bool,
) -> Result<()> {
    let brushes = Brushes::new(target, state, theme)?;
    let accent = accent_for_theme(state, theme);
    draw_background_layer(target, theme, glass_enabled)?;

    draw_surface_layer(target, &brushes, theme, accent)?;
    draw_glass_text_scrims(target, theme, glass_enabled)?;
    draw_header_content(target, formats, &brushes, state, locale, theme)?;
    draw_hero_content(target, dwrite, formats, &brushes, state, locale, theme, accent)?;
    draw_metrics_content(target, dwrite, formats, &brushes, state, locale, theme)?;
    draw_footer_content(target, formats, &brushes, state, locale);
    Ok(())
}

fn draw_background_layer(
    target: &ID2D1DeviceContext,
    theme: Theme,
    glass_enabled: bool,
) -> Result<()> {
    unsafe {
        target.Clear(Some(&if glass_enabled {
            color_alpha(rgb(0, 0, 0), 0.0)
        } else {
            color(theme.background)
        }));
    }
    let (start, end, alpha) = if glass_enabled {
        if theme.dark {
            (rgb(19, 22, 27), theme.background, 0.34)
        } else {
            (rgb(250, 250, 247), theme.background, 0.40)
        }
    } else {
        (if theme.dark { rgb(19, 22, 27) } else { rgb(250, 250, 247) }, theme.background, 1.0)
    };
    let background = linear_gradient_alpha(
        target,
        start,
        end,
        alpha,
        Vector2 { X: 0.0, Y: 0.0 },
        Vector2 { X: 420.0, Y: 430.0 },
    )?;
    unsafe {
        target.FillRectangle(&rect(0.0, 0.0, 420.0, 430.0), &background);
    }
    Ok(())
}

fn draw_surface_layer(
    target: &ID2D1DeviceContext,
    brushes: &Brushes,
    theme: Theme,
    accent_color: COLORREF,
) -> Result<()> {
    let hero = rounded_rect(16.0, 68.0, 404.0, 280.0, 18.0);
    let hero_tint = mix_color(theme.surface, accent_color, if theme.dark { 0.075 } else { 0.035 });
    let hero_gradient = linear_gradient_alpha_pair(
        target,
        hero_tint,
        theme.surface,
        if theme.high_contrast {
            1.0
        } else if theme.dark {
            0.76
        } else {
            0.86
        },
        if theme.high_contrast {
            1.0
        } else if theme.dark {
            0.66
        } else {
            0.78
        },
        Vector2 { X: 20.0, Y: 76.0 },
        Vector2 { X: 398.0, Y: 266.0 },
    )?;
    let metrics = rounded_rect(16.0, 294.0, 404.0, 390.0, 18.0);
    let metric_tint = if theme.dark {
        mix_color(theme.surface_alt, theme.surface, 0.30)
    } else {
        mix_color(theme.surface_alt, theme.surface, 0.62)
    };
    let metric_gradient = linear_gradient_alpha_pair(
        target,
        metric_tint,
        theme.surface,
        if theme.high_contrast {
            1.0
        } else if theme.dark {
            0.70
        } else {
            0.82
        },
        if theme.high_contrast {
            1.0
        } else if theme.dark {
            0.60
        } else {
            0.74
        },
        Vector2 { X: 20.0, Y: 302.0 },
        Vector2 { X: 400.0, Y: 394.0 },
    )?;
    if !theme.high_contrast {
        let mask = record_mask(target, |context, brush| unsafe {
            context.FillRoundedRectangle(&hero, brush);
            context.FillRoundedRectangle(&metrics, brush);
        })?;
        let shadow = unsafe { target.CreateEffect(&CLSID_D2D1Shadow)? };
        // The cards sit 16 DIPs from the window edge; sigma stays near 5 so
        // the effect's ~3σ expansion has real padding instead of being clipped.
        let sigma = if theme.dark { 5.2_f32 } else { 4.8_f32 };
        let shadow_color =
            if theme.dark { [0.0_f32, 0.0, 0.0, 0.52] } else { [0.10_f32, 0.14, 0.12, 0.28] };
        unsafe {
            shadow.SetInput(0, &mask, true);
            shadow.SetValue(
                D2D1_SHADOW_PROP_BLUR_STANDARD_DEVIATION.0 as u32,
                D2D1_PROPERTY_TYPE_FLOAT,
                bytes_of(&sigma),
            )?;
            shadow.SetValue(
                D2D1_SHADOW_PROP_COLOR.0 as u32,
                D2D1_PROPERTY_TYPE_VECTOR4,
                bytes_of(&shadow_color),
            )?;
            let output = shadow.GetOutput()?;
            target.DrawImage(
                &output,
                Some(&Vector2 { X: 0.0, Y: 4.0 }),
                None,
                D2D1_INTERPOLATION_MODE_LINEAR,
                D2D1_COMPOSITE_MODE_SOURCE_OVER,
            );
        }
    }
    unsafe {
        target.FillRoundedRectangle(&hero, &hero_gradient);
        target.FillRoundedRectangle(&metrics, &metric_gradient);
        if theme.high_contrast {
            target.DrawRoundedRectangle(&hero, &brushes.line, 1.0, None::<&ID2D1StrokeStyle>);
            target.DrawRoundedRectangle(&metrics, &brushes.line, 1.0, None::<&ID2D1StrokeStyle>);
        }
    }
    Ok(())
}

fn draw_glass_text_scrims(
    target: &ID2D1DeviceContext,
    theme: Theme,
    glass_enabled: bool,
) -> Result<()> {
    if !glass_enabled || theme.high_contrast {
        return Ok(());
    }

    let scrim_color = if theme.dark { rgb(7, 9, 12) } else { rgb(255, 255, 252) };
    let scrim = solid_brush_alpha(target, scrim_color, if theme.dark { 0.24 } else { 0.30 })?;
    unsafe {
        target.FillRoundedRectangle(&rounded_rect(12.0, 8.0, 367.0, 54.0, 14.0), &scrim);
        target.FillRoundedRectangle(&rounded_rect(15.0, 394.0, 405.0, 428.0, 12.0), &scrim);
    }
    Ok(())
}

fn draw_header_content(
    target: &ID2D1DeviceContext,
    formats: &FormatSet,
    brushes: &Brushes,
    state: &DisplayState,
    locale: Locale,
    theme: Theme,
) -> Result<()> {
    unsafe {
        if !theme.high_contrast {
            target.FillEllipse(&ellipse(24.0, 31.0, 11.0), &brushes.accent_glow_soft);
            target.FillEllipse(&ellipse(24.0, 31.0, 7.0), &brushes.accent_glow);
        }
        target.FillEllipse(&ellipse(24.0, 31.0, 4.2), &brushes.accent);
        draw_text(
            target,
            "CodexStatus",
            rect(39.0, 10.0, 202.0, 52.0),
            &formats.header,
            &brushes.text,
        );
        draw_text(
            target,
            &updated_text(state, locale),
            rect(205.0, 11.0, 365.0, 51.0),
            &formats.update,
            &brushes.muted,
        );

        draw_ring_arrow(target, 393.0, 30.0, 8.2, &brushes.muted, 1.8)?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn draw_hero_content(
    target: &ID2D1DeviceContext,
    dwrite: &IDWriteFactory,
    formats: &FormatSet,
    brushes: &Brushes,
    state: &DisplayState,
    locale: Locale,
    theme: Theme,
    accent_color: COLORREF,
) -> Result<()> {
    unsafe {
        draw_text(
            target,
            locale.text("Weekly remaining", "本周剩余"),
            rect(34.0, 82.0, 210.0, 115.0),
            &formats.label,
            &brushes.muted,
        );
    }
    draw_percentage(target, dwrite, formats, brushes, state.weekly_percent(), theme)?;

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
        draw_text(
            target,
            locale.text("Reset in", "距离重置"),
            rect(239.0, 82.0, 383.0, 115.0),
            &formats.label,
            &brushes.muted,
        );
        draw_text(
            target,
            &reset.0,
            rect(239.0, 109.0, 385.0, 146.0),
            &formats.reset_value,
            &brushes.text,
        );
        draw_text(
            target,
            &reset.1,
            rect(239.0, 143.0, 385.0, 171.0),
            &formats.secondary,
            &brushes.muted,
        );
    }

    draw_quota_track(target, formats, brushes, state, locale, theme, accent_color)?;
    Ok(())
}

fn draw_percentage(
    target: &ID2D1DeviceContext,
    dwrite: &IDWriteFactory,
    formats: &FormatSet,
    brushes: &Brushes,
    percent: Option<u8>,
    theme: Theme,
) -> Result<()> {
    let number = percent.map_or_else(|| "--".to_owned(), |value| value.to_string());
    let width = text_width(dwrite, &number, &formats.quota, 170.0, 80.0)?;
    let number_rect = rect(33.0, 105.0, 207.0, 191.0);
    if percent.is_some() && !theme.high_contrast {
        let mask = record_mask(target, |context, _| unsafe {
            draw_text(context, &number, number_rect, &formats.quota, &brushes.accent);
        })?;
        draw_blurred_mask(target, &mask, 8.5)?;
    }
    unsafe {
        draw_text(
            target,
            &number,
            number_rect,
            &formats.quota,
            if percent.is_some() { &brushes.accent } else { &brushes.text },
        );
        if percent.is_some() {
            draw_text(
                target,
                "%",
                rect(36.0 + width, 125.0, 213.0, 184.0),
                &formats.percent,
                &brushes.accent,
            );
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn draw_quota_track(
    target: &ID2D1DeviceContext,
    formats: &FormatSet,
    brushes: &Brushes,
    state: &DisplayState,
    locale: Locale,
    theme: Theme,
    accent_color: COLORREF,
) -> Result<()> {
    let left = 34.0;
    let right = 386.0;
    let top = 200.0;
    let bottom = 208.0;
    unsafe {
        target.FillRoundedRectangle(&rounded_rect(left, top, right, bottom, 4.0), &brushes.track);
    }

    if let Some(value) = state.weekly_percent()
        && value > 0
    {
        let filled = (left + (right - left) * f32::from(value) / 100.0).clamp(left + 8.0, right);
        let start_color =
            mix_color(accent_color, theme.surface, if theme.dark { 0.12 } else { 0.20 });
        let progress = linear_gradient(
            target,
            start_color,
            accent_color,
            Vector2 { X: left, Y: top },
            Vector2 { X: right, Y: top },
        )?;
        unsafe {
            target.FillRoundedRectangle(&rounded_rect(left, top, filled, bottom, 4.0), &progress);
            if value < 100 {
                if !theme.high_contrast {
                    let mask = record_mask(target, |context, _| {
                        context.FillEllipse(&ellipse(filled, 204.0, 5.0), &brushes.accent);
                    })?;
                    draw_blurred_mask(target, &mask, 6.0)?;
                }
                target.FillEllipse(&ellipse(filled, 204.0, 4.5), &brushes.accent);
            }
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
    if let Some(insight) = pace_insight {
        if let Some(elapsed) = insight.elapsed_percent {
            let expected_remaining = (100.0 - elapsed).clamp(0.0, 100.0) as f32;
            let marker = left + (right - left) * expected_remaining / 100.0;
            unsafe {
                target.DrawLine(
                    Vector2 { X: marker, Y: top - 2.0 },
                    Vector2 { X: marker, Y: bottom + 2.0 },
                    &brushes.muted,
                    1.0,
                    None::<&ID2D1StrokeStyle>,
                );
            }
        }
    }
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
    unsafe {
        draw_text(
            target,
            &pace_text,
            rect(34.0, 221.0, 386.0, 263.0),
            &formats.pace,
            match projection {
                Some(UsageProjection::Exhausted) => &brushes.error,
                Some(UsageProjection::DepletesIn { .. }) => &brushes.warning,
                None => &brushes.text,
            },
        );
    }
    Ok(())
}

fn draw_metrics_content(
    target: &ID2D1DeviceContext,
    dwrite: &IDWriteFactory,
    formats: &FormatSet,
    brushes: &Brushes,
    state: &DisplayState,
    locale: Locale,
    theme: Theme,
) -> Result<()> {
    let session = state
        .snapshot
        .as_ref()
        .and_then(|snapshot| snapshot.session.as_ref())
        .map(|window| format!("{}%", window.display_percent()));
    let plan_type =
        state.snapshot.as_ref().and_then(|snapshot| snapshot.account.plan_type.as_deref());
    let plan = plan_type.map(|plan| plan_label(plan, locale)).unwrap_or("--").to_owned();
    let credits = state
        .snapshot
        .as_ref()
        .and_then(|snapshot| snapshot.account.reset_credits)
        .map(|credits| format!("{credits} {}", locale.text("resets", "次")))
        .unwrap_or_else(|| "--".to_owned());

    if let Some(session) = session {
        draw_plan_metric(
            target,
            dwrite,
            formats,
            brushes,
            rect(17.0, 295.0, 146.0, 389.0),
            plan_type,
            &plan,
            theme,
        )?;
        draw_metric(
            target,
            formats,
            brushes,
            rect(147.0, 295.0, 274.0, 389.0),
            locale.text("5-hour", "5 小时"),
            &session,
        );
        draw_metric(
            target,
            formats,
            brushes,
            rect(275.0, 295.0, 403.0, 389.0),
            locale.text("Reset credits", "重置机会"),
            &credits,
        );
    } else {
        draw_plan_metric(
            target,
            dwrite,
            formats,
            brushes,
            rect(17.0, 295.0, 220.0, 389.0),
            plan_type,
            &plan,
            theme,
        )?;
        draw_metric(
            target,
            formats,
            brushes,
            rect(221.0, 295.0, 403.0, 389.0),
            locale.text("Reset credits", "重置机会"),
            &credits,
        );
        draw_ring_arrow(target, 371.0, 351.0, 10.5, &brushes.muted, 2.0)?;
    }
    Ok(())
}

fn draw_metric(
    target: &ID2D1DeviceContext,
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
            rect(area.left + 17.0, area.top + 9.0, area.right - 13.0, area.top + 43.0),
            &formats.metric_label,
            &brushes.muted,
        );
        draw_text(
            target,
            value,
            rect(area.left + 17.0, area.top + 42.0, area.right - 13.0, area.bottom - 8.0),
            &formats.metric_value,
            &brushes.text,
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_plan_metric(
    target: &ID2D1DeviceContext,
    dwrite: &IDWriteFactory,
    formats: &FormatSet,
    brushes: &Brushes,
    area: D2D_RECT_F,
    plan_type: Option<&str>,
    plan: &str,
    theme: Theme,
) -> Result<()> {
    unsafe {
        draw_text(
            target,
            if formats.locale == Locale::Chinese { "套餐" } else { "Plan" },
            rect(area.left + 17.0, area.top + 8.0, area.right - 13.0, area.top + 41.0),
            &formats.metric_label,
            &brushes.muted,
        );
    }
    draw_plan_badge(
        target,
        dwrite,
        formats,
        brushes,
        plan_type,
        plan,
        area.left + 17.0,
        area.top + 43.0,
        (area.right - area.left - 34.0).max(58.0),
        theme,
    )
}

#[allow(clippy::too_many_arguments)]
fn draw_plan_badge(
    target: &ID2D1DeviceContext,
    dwrite: &IDWriteFactory,
    formats: &FormatSet,
    brushes: &Brushes,
    plan_type: Option<&str>,
    plan: &str,
    x: f32,
    y: f32,
    max_width: f32,
    theme: Theme,
) -> Result<()> {
    let tier = plan_badge_tier(plan_type);
    let text_width = text_width(dwrite, plan, &formats.badge, max_width, 32.0)?;
    let width = (text_width + 46.0).clamp(66.0, max_width);
    let badge = rounded_rect(x, y, x + width, y + 32.0, 11.0);
    let (start, end, start_alpha, end_alpha) = match tier {
        1 => (theme.surface_alt, theme.surface, 0.72, 0.48),
        2 => (rgb(36, 150, 121), rgb(67, 196, 154), 0.78, 0.58),
        3 => (rgb(44, 112, 186), rgb(77, 145, 218), 0.82, 0.64),
        4 => (rgb(86, 83, 196), rgb(145, 96, 211), 0.86, 0.68),
        5 => (rgb(37, 43, 68), rgb(111, 72, 165), 0.90, 0.70),
        _ => (theme.surface_alt, theme.surface, 0.66, 0.44),
    };
    if theme.high_contrast {
        unsafe {
            target.DrawRoundedRectangle(&badge, &brushes.text, 1.5, None::<&ID2D1StrokeStyle>);
        }
    } else {
        let fill = linear_gradient_alpha_pair(
            target,
            start,
            end,
            start_alpha,
            end_alpha,
            Vector2 { X: x, Y: y },
            Vector2 { X: x + width, Y: y + 32.0 },
        )?;
        unsafe {
            target.FillRoundedRectangle(&badge, &fill);
            target.DrawRoundedRectangle(&badge, &brushes.line, 1.0, None::<&ID2D1StrokeStyle>);
        }
    }

    let chip_foreground = solid_brush(target, rgb(250, 252, 250))?;
    let marker_brush =
        if theme.high_contrast || tier <= 1 { &brushes.text } else { &chip_foreground };
    for index in 0..5 {
        let left = x + 9.0 + index as f32 * 3.6;
        let height = 4.0 + index as f32 * 1.8;
        let marker = rounded_rect(left, y + 22.0 - height, left + 2.2, y + 22.0, 1.1);
        unsafe {
            if tier == 0 || index >= tier {
                target.DrawRoundedRectangle(&marker, marker_brush, 0.8, None::<&ID2D1StrokeStyle>);
            } else {
                target.FillRoundedRectangle(&marker, marker_brush);
            }
        }
    }
    unsafe {
        draw_text(
            target,
            plan,
            rect(x + 31.0, y - 1.0, x + width - 7.0, y + 33.0),
            &formats.badge,
            if theme.high_contrast || tier <= 1 { &brushes.text } else { &chip_foreground },
        );
    }
    Ok(())
}

fn plan_badge_tier(plan_type: Option<&str>) -> u8 {
    match plan_type.map(str::to_ascii_lowercase).as_deref() {
        Some("free") => 1_u8,
        Some("go") => 2,
        Some("plus") => 3,
        Some("prolite") => 4,
        Some("pro") => 5,
        _ => 0,
    }
}

fn draw_footer_content(
    target: &ID2D1DeviceContext,
    formats: &FormatSet,
    brushes: &Brushes,
    state: &DisplayState,
    locale: Locale,
) {
    let footer_brush = if state.error.is_some() { &brushes.error } else { &brushes.muted };
    let dot_brush = if state.error.is_some() { &brushes.error } else { &brushes.accent };
    unsafe {
        target.FillEllipse(&ellipse(24.0, 411.0, 3.0), dot_brush);
        draw_text(
            target,
            &footer_text(state, locale),
            rect(34.0, 395.0, 402.0, 426.0),
            &formats.footer,
            footer_brush,
        );
    }
}

unsafe fn draw_text(
    target: &ID2D1DeviceContext,
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

fn solid_brush(target: &ID2D1DeviceContext, value: COLORREF) -> Result<ID2D1SolidColorBrush> {
    unsafe { target.CreateSolidColorBrush(&color(value), None) }
}

fn solid_brush_alpha(
    target: &ID2D1DeviceContext,
    value: COLORREF,
    alpha: f32,
) -> Result<ID2D1SolidColorBrush> {
    unsafe { target.CreateSolidColorBrush(&color_alpha(value, alpha), None) }
}

fn record_mask(
    target: &ID2D1DeviceContext,
    draw: impl FnOnce(&ID2D1DeviceContext, &ID2D1SolidColorBrush),
) -> Result<ID2D1CommandList> {
    unsafe {
        let original = target.GetTarget()?;
        let command_list = target.CreateCommandList()?;
        let mask = target.CreateSolidColorBrush(&color_alpha(rgb(255, 255, 255), 1.0), None)?;
        target.SetTarget(&command_list);
        target.Clear(Some(&color_alpha(rgb(0, 0, 0), 0.0)));
        draw(target, &mask);
        target.SetTarget(&original);
        command_list.Close()?;
        Ok(command_list)
    }
}

fn draw_blurred_mask(
    target: &ID2D1DeviceContext,
    mask: &ID2D1CommandList,
    sigma: f32,
) -> Result<()> {
    let blur = unsafe { target.CreateEffect(&CLSID_D2D1GaussianBlur)? };
    unsafe {
        blur.SetInput(0, mask, true);
        blur.SetValue(
            D2D1_GAUSSIANBLUR_PROP_STANDARD_DEVIATION.0 as u32,
            D2D1_PROPERTY_TYPE_FLOAT,
            bytes_of(&sigma),
        )?;
        let output = blur.GetOutput()?;
        target.DrawImage(
            &output,
            None,
            None,
            D2D1_INTERPOLATION_MODE_LINEAR,
            D2D1_COMPOSITE_MODE_SOURCE_OVER,
        );
    }
    Ok(())
}

fn draw_ring_arrow(
    target: &ID2D1DeviceContext,
    x: f32,
    y: f32,
    radius: f32,
    brush: &ID2D1SolidColorBrush,
    stroke: f32,
) -> Result<()> {
    // A 292° clockwise arc leaves a deliberate 68° gap. The arrowhead is
    // proportional to the radius and filled, so it remains a triangle rather
    // than collapsing into a tiny hooked stroke at high DPI.
    let start_angle = 28.0_f32.to_radians();
    let sweep = 292.0_f32.to_radians();
    let segment_count = (radius * 2.2).round().clamp(18.0, 34.0) as usize;
    let point = |angle: f32| Vector2 { X: x + radius * angle.cos(), Y: y + radius * angle.sin() };
    let mut previous = point(start_angle);
    unsafe {
        for index in 1..=segment_count {
            let angle = start_angle + sweep * index as f32 / segment_count as f32;
            let next = point(angle);
            target.DrawLine(previous, next, brush, stroke, None::<&ID2D1StrokeStyle>);
            previous = next;
        }

        let tangent = Vector2 { X: -(start_angle + sweep).sin(), Y: (start_angle + sweep).cos() };
        let normal = Vector2 { X: -tangent.Y, Y: tangent.X };
        let head_length = (radius * 0.72).clamp(5.2, 8.0);
        let half_width = (radius * 0.43).clamp(3.2, 5.0);
        let tip = Vector2 {
            X: previous.X + tangent.X * head_length * 0.72,
            Y: previous.Y + tangent.Y * head_length * 0.72,
        };
        let base_center = Vector2 {
            X: previous.X - tangent.X * head_length * 0.38,
            Y: previous.Y - tangent.Y * head_length * 0.38,
        };
        let base_a = Vector2 {
            X: base_center.X + normal.X * half_width,
            Y: base_center.Y + normal.Y * half_width,
        };
        let base_b = Vector2 {
            X: base_center.X - normal.X * half_width,
            Y: base_center.Y - normal.Y * half_width,
        };

        let factory = target.GetFactory()?;
        let triangle = factory.CreatePathGeometry()?;
        let sink = triangle.Open()?;
        sink.BeginFigure(tip, D2D1_FIGURE_BEGIN_FILLED);
        sink.AddLine(base_a);
        sink.AddLine(base_b);
        sink.EndFigure(D2D1_FIGURE_END_CLOSED);
        sink.Close()?;
        target.FillGeometry(&triangle, brush, None::<&ID2D1Brush>);
    }
    Ok(())
}

fn linear_gradient(
    target: &ID2D1DeviceContext,
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
        let collection = target.CreateGradientStopCollection(
            &stops,
            D2D1_COLOR_SPACE_SRGB,
            D2D1_COLOR_SPACE_SRGB,
            D2D1_BUFFER_PRECISION_8BPC_UNORM,
            D2D1_EXTEND_MODE_CLAMP,
            D2D1_COLOR_INTERPOLATION_MODE_STRAIGHT,
        )?;
        let properties = D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES { startPoint: start, endPoint: end };
        target.CreateLinearGradientBrush(&properties, None, &collection)
    }
}

#[allow(clippy::too_many_arguments)]
fn linear_gradient_alpha_pair(
    target: &ID2D1DeviceContext,
    start_color: COLORREF,
    end_color: COLORREF,
    start_alpha: f32,
    end_alpha: f32,
    start: Vector2,
    end: Vector2,
) -> Result<ID2D1LinearGradientBrush> {
    let stops = [
        D2D1_GRADIENT_STOP { position: 0.0, color: color_alpha(start_color, start_alpha) },
        D2D1_GRADIENT_STOP { position: 1.0, color: color_alpha(end_color, end_alpha) },
    ];
    unsafe {
        let collection = target.CreateGradientStopCollection(
            &stops,
            D2D1_COLOR_SPACE_SRGB,
            D2D1_COLOR_SPACE_SRGB,
            D2D1_BUFFER_PRECISION_8BPC_UNORM,
            D2D1_EXTEND_MODE_CLAMP,
            D2D1_COLOR_INTERPOLATION_MODE_STRAIGHT,
        )?;
        let properties = D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES { startPoint: start, endPoint: end };
        target.CreateLinearGradientBrush(&properties, None, &collection)
    }
}

fn linear_gradient_alpha(
    target: &ID2D1DeviceContext,
    start_color: COLORREF,
    end_color: COLORREF,
    alpha: f32,
    start: Vector2,
    end: Vector2,
) -> Result<ID2D1LinearGradientBrush> {
    let stops = [
        D2D1_GRADIENT_STOP { position: 0.0, color: color_alpha(start_color, alpha) },
        D2D1_GRADIENT_STOP { position: 1.0, color: color_alpha(end_color, alpha) },
    ];
    unsafe {
        let collection = target.CreateGradientStopCollection(
            &stops,
            D2D1_COLOR_SPACE_SRGB,
            D2D1_COLOR_SPACE_SRGB,
            D2D1_BUFFER_PRECISION_8BPC_UNORM,
            D2D1_EXTEND_MODE_CLAMP,
            D2D1_COLOR_INTERPOLATION_MODE_STRAIGHT,
        )?;
        let properties = D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES { startPoint: start, endPoint: end };
        target.CreateLinearGradientBrush(&properties, None, &collection)
    }
}

fn accent_for_theme(state: &DisplayState, theme: Theme) -> COLORREF {
    if theme.high_contrast {
        return theme.text;
    }
    match state.weekly_percent() {
        Some(value) if value < 20 => {
            if theme.dark {
                rgb(255, 113, 129)
            } else {
                rgb(218, 58, 76)
            }
        }
        Some(value) if value < 50 => {
            if theme.dark {
                rgb(255, 193, 83)
            } else {
                rgb(205, 128, 15)
            }
        }
        Some(_) => {
            if theme.dark {
                rgb(50, 238, 137)
            } else {
                rgb(18, 196, 105)
            }
        }
        None => {
            if theme.dark {
                rgb(139, 157, 168)
            } else {
                rgb(92, 116, 128)
            }
        }
    }
}

fn color(value: COLORREF) -> D2D1_COLOR_F {
    color_alpha(value, 1.0)
}

fn color_alpha(value: COLORREF, alpha: f32) -> D2D1_COLOR_F {
    D2D1_COLOR_F {
        r: (value.0 & 0xff) as f32 / 255.0,
        g: ((value.0 >> 8) & 0xff) as f32 / 255.0,
        b: ((value.0 >> 16) & 0xff) as f32 / 255.0,
        a: alpha.clamp(0.0, 1.0),
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

fn bytes_of<T>(value: &T) -> &[u8] {
    unsafe { std::slice::from_raw_parts((value as *const T).cast::<u8>(), size_of::<T>()) }
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
        for surface in [
            rounded_rect(16.0, 68.0, 404.0, 282.0, 19.0),
            rounded_rect(16.0, 298.0, 404.0, 397.0, 19.0),
        ] {
            assert!(surface.rect.left >= 0.0);
            assert!(surface.rect.right <= super::super::CARD_WIDTH as f32);
            assert!(surface.rect.bottom <= super::super::CARD_HEIGHT as f32);
        }
    }

    #[test]
    fn plan_badge_tiers_cover_personal_plans_and_keep_unknown_fallback() {
        assert_eq!(plan_badge_tier(Some("free")), 1);
        assert_eq!(plan_badge_tier(Some("go")), 2);
        assert_eq!(plan_badge_tier(Some("plus")), 3);
        assert_eq!(plan_badge_tier(Some("prolite")), 4);
        assert_eq!(plan_badge_tier(Some("pro")), 5);
        assert_eq!(plan_badge_tier(Some("enterprise")), 0);
        assert_eq!(plan_badge_tier(None), 0);
    }
}
