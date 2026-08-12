//! Direct2D/DirectWrite renderer for the CodexStatus flyout.
//!
//! THESIS: Native Aurora Lite is one compact monitor surface, not a stack of glass cards.
//! OWN-WORLD: clipped backdrop blur, one quiet content plane, blurred cyan/rose/apricot light.
//! STORY: read remaining quota first, compare reset timing, then scan supporting facts.
//! FIRST VIEWPORT: a 420×412 flyout with a two-column hero and one integrated facts row.
//! FORM: approved Operate-mode single-surface composition; status color stays semantic.

use super::composition::{DesktopComposition, WinRtApartment};
use super::{
    HEADER_BRAND, Locale, RefreshButtonState, Theme, accent_for, accent_red, credit_expiry_text,
    footer_primary, pace_label, plan_label, refresh_button_fill, reset_details, rgb, updated_text,
};
use crate::insights::analyze_window;
use crate::model::DisplayState;
use chrono::Local;
use std::cell::RefCell;
use std::mem::{ManuallyDrop, size_of};
use windows::Win32::Foundation::{COLORREF, D2DERR_RECREATE_TARGET, HMODULE, HWND};
use windows::Win32::Graphics::Direct2D::Common::{
    D2D_RECT_F, D2D_SIZE_U, D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_COLOR_F,
    D2D1_COMPOSITE_MODE_SOURCE_OVER, D2D1_FIGURE_BEGIN_FILLED, D2D1_FIGURE_END_CLOSED,
    D2D1_FILL_MODE_ALTERNATE, D2D1_GRADIENT_STOP, D2D1_PIXEL_FORMAT,
};
use windows::Win32::Graphics::Direct2D::{
    CLSID_D2D1GaussianBlur, CLSID_D2D1Saturation, CLSID_D2D1Shadow,
    D2D1_ANTIALIAS_MODE_PER_PRIMITIVE, D2D1_BITMAP_OPTIONS_CANNOT_DRAW, D2D1_BITMAP_OPTIONS_TARGET,
    D2D1_BITMAP_PROPERTIES1, D2D1_BUFFER_PRECISION_8BPC_UNORM,
    D2D1_COLOR_INTERPOLATION_MODE_STRAIGHT, D2D1_COLOR_SPACE_SRGB,
    D2D1_DEVICE_CONTEXT_OPTIONS_NONE, D2D1_DRAW_TEXT_OPTIONS_NONE, D2D1_EXTEND_MODE_CLAMP,
    D2D1_FACTORY_TYPE_SINGLE_THREADED, D2D1_GAUSSIANBLUR_PROP_STANDARD_DEVIATION,
    D2D1_INTERPOLATION_MODE_LINEAR, D2D1_LAYER_OPTIONS1_NONE, D2D1_LAYER_PARAMETERS1,
    D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES, D2D1_PROPERTY_TYPE_FLOAT, D2D1_PROPERTY_TYPE_VECTOR4,
    D2D1_RADIAL_GRADIENT_BRUSH_PROPERTIES, D2D1_ROUNDED_RECT, D2D1_SATURATION_PROP_SATURATION,
    D2D1_SHADOW_PROP_BLUR_STANDARD_DEVIATION, D2D1_SHADOW_PROP_COLOR,
    D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE, D2D1CreateFactory, ID2D1Bitmap1, ID2D1Brush,
    ID2D1CommandList, ID2D1Device, ID2D1DeviceContext, ID2D1Factory1, ID2D1Geometry, ID2D1Image,
    ID2D1LinearGradientBrush, ID2D1RadialGradientBrush, ID2D1SolidColorBrush, ID2D1StrokeStyle,
};
use windows::Win32::Graphics::Direct3D::{
    D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_WARP, D3D_FEATURE_LEVEL_11_0,
};
use windows::Win32::Graphics::Direct3D11::{
    D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_SDK_VERSION, D3D11CreateDevice, ID3D11Device,
};
use windows::Win32::Graphics::DirectWrite::{
    DWRITE_FACTORY_TYPE_SHARED, DWRITE_FONT_FEATURE, DWRITE_FONT_FEATURE_TAG_TABULAR_FIGURES,
    DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_WEIGHT,
    DWRITE_FONT_WEIGHT_NORMAL, DWRITE_FONT_WEIGHT_SEMI_BOLD, DWRITE_MEASURING_MODE_NATURAL,
    DWRITE_PARAGRAPH_ALIGNMENT_CENTER, DWRITE_TEXT_ALIGNMENT, DWRITE_TEXT_ALIGNMENT_LEADING,
    DWRITE_TEXT_ALIGNMENT_TRAILING, DWRITE_TEXT_METRICS, DWRITE_TEXT_RANGE, DWRITE_TRIMMING,
    DWRITE_TRIMMING_GRANULARITY_CHARACTER, DWRITE_WORD_WRAPPING_NO_WRAP, DWriteCreateFactory,
    IDWriteFactory, IDWriteFontCollection, IDWriteTextFormat, IDWriteTextLayout, IDWriteTypography,
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
use windows_numerics::{Matrix3x2, Vector2};

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
    pub refresh_button: RefreshButtonState,
    pub refreshing: bool,
    pub refresh_angle_degrees: f32,
    pub glass_enabled: bool,
}

pub(super) fn paint(input: PaintInput<'_>) -> bool {
    #[cfg(feature = "diagnostics")]
    if std::env::var_os("CODEX_STATUS_DIAGNOSTIC_FORCE_GDI").is_some() {
        activate_gdi_fallback(input.hwnd);
        return false;
    }
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
    _winrt_apartment: WinRtApartment,
}

impl Renderer {
    fn new() -> Result<Self> {
        unsafe {
            let winrt_apartment = WinRtApartment::initialize()?;
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
                _winrt_apartment: winrt_apartment,
            })
        }
    }

    fn paint(&mut self, input: &PaintInput<'_>) -> Result<()> {
        self.ensure_surface(input.hwnd, input.size.0, input.size.1, input.dpi)?;
        self.surface
            .as_ref()
            .expect("surface initialized")
            .composition
            .set_glass_enabled(input.glass_enabled)?;
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
            input.refresh_button,
            input.refreshing,
            input.refresh_angle_degrees,
            input.glass_enabled,
        );
        let end_result = unsafe { context.EndDraw(None, None) };
        end_result?;
        draw_result?;

        let surface = self.surface.as_ref().expect("surface initialized");
        unsafe {
            surface.swap_chain.Present(1, DXGI_PRESENT(0)).ok()?;
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
    composition: DesktopComposition,
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
            let target_bitmap = create_target_bitmap(context, &swap_chain, dpi)?;
            let composition = DesktopComposition::new(
                hwnd,
                &swap_chain,
                pixel_size.width,
                pixel_size.height,
                dpi,
            )?;

            context.SetTarget(&target_bitmap);
            context.SetDpi(dpi as f32, dpi as f32);

            Ok(Self {
                hwnd,
                swap_chain,
                composition,
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
            self.composition.resize(pixel_size.width, pixel_size.height, dpi)?;
            self.pixel_size = pixel_size;
            self.dpi = dpi;
        }
        Ok(())
    }

    fn detach(self) -> Result<()> {
        self.composition.detach()
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
    tabular: IDWriteTypography,
    header: IDWriteTextFormat,
    update: IDWriteTextFormat,
    label: IDWriteTextFormat,
    label_trailing: IDWriteTextFormat,
    quota: IDWriteTextFormat,
    quota_suffix: IDWriteTextFormat,
    reset_value: IDWriteTextFormat,
    secondary: IDWriteTextFormat,
    pace: IDWriteTextFormat,
    pace_status: IDWriteTextFormat,
    metric_label: IDWriteTextFormat,
    metric_value: IDWriteTextFormat,
    footer: IDWriteTextFormat,
}

impl FormatSet {
    fn new(factory: &IDWriteFactory, family: PCWSTR, locale: Locale) -> Result<Self> {
        let tabular = unsafe { factory.CreateTypography()? };
        unsafe {
            tabular.AddFontFeature(DWRITE_FONT_FEATURE {
                nameTag: DWRITE_FONT_FEATURE_TAG_TABULAR_FIGURES,
                parameter: 1,
            })?;
        }
        Ok(Self {
            locale,
            tabular,
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
                12.0,
                DWRITE_FONT_WEIGHT_NORMAL,
                DWRITE_TEXT_ALIGNMENT_TRAILING,
                true,
            )?,
            label: make_format(
                factory,
                family,
                locale,
                14.0,
                DWRITE_FONT_WEIGHT_NORMAL,
                DWRITE_TEXT_ALIGNMENT_LEADING,
                true,
            )?,
            label_trailing: make_format(
                factory,
                family,
                locale,
                14.0,
                DWRITE_FONT_WEIGHT_NORMAL,
                DWRITE_TEXT_ALIGNMENT_TRAILING,
                true,
            )?,
            quota: make_format(
                factory,
                family,
                locale,
                68.0,
                DWRITE_FONT_WEIGHT_SEMI_BOLD,
                DWRITE_TEXT_ALIGNMENT_LEADING,
                false,
            )?,
            quota_suffix: make_format(
                factory,
                family,
                locale,
                20.0,
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
                DWRITE_TEXT_ALIGNMENT_TRAILING,
                true,
            )?,
            secondary: make_format(
                factory,
                family,
                locale,
                12.0,
                DWRITE_FONT_WEIGHT_NORMAL,
                DWRITE_TEXT_ALIGNMENT_TRAILING,
                true,
            )?,
            pace: make_format(
                factory,
                family,
                locale,
                14.0,
                DWRITE_FONT_WEIGHT_NORMAL,
                DWRITE_TEXT_ALIGNMENT_LEADING,
                true,
            )?,
            pace_status: make_format(
                factory,
                family,
                locale,
                12.0,
                DWRITE_FONT_WEIGHT_SEMI_BOLD,
                DWRITE_TEXT_ALIGNMENT_TRAILING,
                true,
            )?,
            metric_label: make_format(
                factory,
                family,
                locale,
                12.0,
                DWRITE_FONT_WEIGHT_NORMAL,
                DWRITE_TEXT_ALIGNMENT_LEADING,
                true,
            )?,
            metric_value: make_format(
                factory,
                family,
                locale,
                20.0,
                DWRITE_FONT_WEIGHT_SEMI_BOLD,
                DWRITE_TEXT_ALIGNMENT_LEADING,
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
    error: ID2D1SolidColorBrush,
    footer_primary: ID2D1SolidColorBrush,
}

impl Brushes {
    fn new(target: &ID2D1DeviceContext, state: &DisplayState, theme: Theme) -> Result<Self> {
        let accent = accent_for(state, theme);
        let error = if theme.high_contrast { theme.text } else { accent_red(theme) };
        Ok(Self {
            text: solid_brush(target, theme.text)?,
            muted: solid_brush(target, theme.muted)?,
            line: solid_brush(target, theme.line)?,
            track: solid_brush(target, theme.line)?,
            accent: solid_brush(target, accent)?,
            error: solid_brush(target, error)?,
            footer_primary: solid_brush(
                target,
                if theme.high_contrast {
                    theme.text
                } else if theme.dark {
                    rgb(190, 197, 194)
                } else {
                    rgb(73, 83, 93)
                },
            )?,
        })
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_frame(
    target: &ID2D1DeviceContext,
    dwrite: &IDWriteFactory,
    formats: &FormatSet,
    state: &DisplayState,
    locale: Locale,
    theme: Theme,
    refresh_button: RefreshButtonState,
    refreshing: bool,
    refresh_angle_degrees: f32,
    glass_enabled: bool,
) -> Result<()> {
    let brushes = Brushes::new(target, state, theme)?;
    let background = draw_background_layer(target, theme, glass_enabled)?;

    draw_surface_layer(target, &brushes, theme, background.as_ref())?;
    draw_header_content(
        target,
        dwrite,
        formats,
        &brushes,
        state,
        theme,
        refresh_button,
        refreshing,
        refresh_angle_degrees,
    )?;
    draw_hero_content(target, dwrite, formats, &brushes, state, locale, theme)?;
    draw_metrics_content(target, dwrite, formats, &brushes, state, locale, theme)?;
    draw_footer_content(target, dwrite, formats, &brushes, state, locale)?;
    Ok(())
}

fn draw_background_layer(
    target: &ID2D1DeviceContext,
    theme: Theme,
    glass_enabled: bool,
) -> Result<Option<ID2D1CommandList>> {
    unsafe {
        target.Clear(Some(&color_alpha(rgb(0, 0, 0), 0.0)));
    }
    if theme.high_contrast {
        unsafe {
            target.Clear(Some(&color(theme.background)));
        }
        return Ok(None);
    }
    let original = unsafe { target.GetTarget()? };
    let material = unsafe { target.CreateCommandList()? };
    unsafe {
        target.SetTarget(&material);
        target.Clear(Some(&color_alpha(rgb(0, 0, 0), 0.0)));
    }

    let alpha = if glass_enabled { if theme.dark { 0.90 } else { 0.76 } } else { 1.0 };
    let background = if theme.dark {
        linear_gradient_three(
            target,
            [(rgb(8, 13, 18), alpha), (rgb(11, 13, 18), alpha), (rgb(8, 12, 16), alpha)],
            Vector2 { X: 0.0, Y: 0.0 },
            Vector2 { X: 420.0, Y: 412.0 },
        )?
    } else {
        linear_gradient_three(
            target,
            [(rgb(232, 237, 241), alpha), (rgb(247, 245, 244), alpha), (rgb(231, 237, 239), alpha)],
            Vector2 { X: 0.0, Y: 0.0 },
            Vector2 { X: 420.0, Y: 412.0 },
        )?
    };
    unsafe { target.FillRectangle(&rect(0.0, 0.0, 420.0, 412.0), &background) };

    draw_broad_aurora(target, theme)?;

    // The compact pools add the controlled chroma visible in the approved design.
    if draw_blurred_aurora(target, theme).is_err() {
        draw_aurora_pools(target, theme)?;
    }

    let glass_sheen = linear_gradient_alpha_pair(
        target,
        rgb(255, 255, 255),
        rgb(255, 255, 255),
        if theme.dark { 0.035 } else { 0.20 },
        0.0,
        Vector2 { X: 34.0, Y: 0.0 },
        Vector2 { X: 330.0, Y: 255.0 },
    )?;
    unsafe {
        target.FillRectangle(&rect(0.0, 0.0, 420.0, 412.0), &glass_sheen);
        let outer_keyline =
            solid_brush_alpha(target, rgb(255, 255, 255), if theme.dark { 0.11 } else { 0.72 })?;
        target.DrawRoundedRectangle(
            &rounded_rect(0.5, 0.5, 419.5, 411.5, 19.5),
            &outer_keyline,
            1.0,
            None::<&ID2D1StrokeStyle>,
        );
        target.SetTarget(&original);
        material.Close()?;
        target.DrawImage(
            &material,
            None,
            None,
            D2D1_INTERPOLATION_MODE_LINEAR,
            D2D1_COMPOSITE_MODE_SOURCE_OVER,
        );
    }
    Ok(Some(material))
}

fn draw_surface_layer(
    target: &ID2D1DeviceContext,
    brushes: &Brushes,
    theme: Theme,
    background: Option<&ID2D1CommandList>,
) -> Result<()> {
    let surface = rounded_rect(14.0, 58.0, 406.0, 366.0, 16.0);

    if !theme.high_contrast {
        draw_surface_shadow(target, &surface, theme)?;
    }

    if !theme.high_contrast
        && let Some(background) = background
    {
        draw_surface_backdrop(target, background, &surface, theme)?;
    }

    let surface_top = if theme.dark { rgb(8, 12, 16) } else { rgb(255, 255, 255) };
    let surface_bottom = if theme.dark { rgb(2, 5, 8) } else { rgb(250, 252, 253) };
    let surface_gradient = linear_gradient_alpha_pair(
        target,
        surface_top,
        surface_bottom,
        if theme.high_contrast {
            1.0
        } else if theme.dark {
            0.07
        } else {
            0.11
        },
        if theme.high_contrast {
            1.0
        } else if theme.dark {
            0.045
        } else {
            0.06
        },
        Vector2 { X: 18.0, Y: 62.0 },
        Vector2 { X: 400.0, Y: 360.0 },
    )?;
    let material_sheen = linear_gradient_alpha_pair(
        target,
        rgb(255, 255, 255),
        rgb(255, 255, 255),
        if theme.high_contrast {
            0.0
        } else if theme.dark {
            0.035
        } else {
            0.10
        },
        0.0,
        Vector2 { X: 24.0, Y: 62.0 },
        Vector2 { X: 344.0, Y: 252.0 },
    )?;
    unsafe {
        target.FillRoundedRectangle(&surface, &surface_gradient);
        target.FillRoundedRectangle(&surface, &material_sheen);
        if theme.high_contrast {
            target.DrawRoundedRectangle(&surface, &brushes.line, 1.0, None::<&ID2D1StrokeStyle>);
        } else {
            let keyline = solid_brush_alpha(
                target,
                rgb(255, 255, 255),
                if theme.dark { 0.15 } else { 0.74 },
            )?;
            target.DrawRoundedRectangle(&surface, &keyline, 1.0, None::<&ID2D1StrokeStyle>);
        }
    }
    Ok(())
}

fn draw_surface_shadow(
    target: &ID2D1DeviceContext,
    surface: &D2D1_ROUNDED_RECT,
    theme: Theme,
) -> Result<()> {
    let original = unsafe { target.GetTarget()? };
    let mask = unsafe { target.CreateCommandList()? };
    unsafe {
        target.SetTarget(&mask);
        target.Clear(Some(&color_alpha(rgb(0, 0, 0), 0.0)));
        let fill = solid_brush(target, rgb(255, 255, 255))?;
        target.FillRoundedRectangle(surface, &fill);
        target.SetTarget(&original);
        mask.Close()?;
    }

    let factory = unsafe { target.GetFactory()? };
    let outer: ID2D1Geometry =
        unsafe { factory.CreateRectangleGeometry(&rect(0.0, 0.0, 420.0, 412.0))?.cast()? };
    let inner: ID2D1Geometry = unsafe { factory.CreateRoundedRectangleGeometry(surface)?.cast()? };
    let outside: ID2D1Geometry = unsafe {
        factory
            .CreateGeometryGroup(D2D1_FILL_MODE_ALTERNATE, &[Some(outer), Some(inner)])?
            .cast()?
    };
    let layer = unsafe { target.CreateLayer(None)? };
    let parameters = D2D1_LAYER_PARAMETERS1 {
        contentBounds: rect(0.0, 0.0, 420.0, 412.0),
        geometricMask: ManuallyDrop::new(Some(outside)),
        maskAntialiasMode: D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
        maskTransform: Matrix3x2 { M11: 1.0, M12: 0.0, M21: 0.0, M22: 1.0, M31: 0.0, M32: 0.0 },
        opacity: 1.0,
        opacityBrush: ManuallyDrop::new(None),
        layerOptions: D2D1_LAYER_OPTIONS1_NONE,
    };
    let (ambient_alpha, contact_alpha) = if theme.dark { (0.46, 0.34) } else { (0.26, 0.20) };
    unsafe { target.PushLayer(&parameters, &layer) };
    let result = draw_shadow_pass(target, &mask, 7.5, ambient_alpha, 6.0)
        .and_then(|()| draw_shadow_pass(target, &mask, 2.2, contact_alpha, 2.0));
    unsafe { target.PopLayer() };
    result
}

fn draw_shadow_pass(
    target: &ID2D1DeviceContext,
    mask: &ID2D1CommandList,
    sigma: f32,
    alpha: f32,
    offset_y: f32,
) -> Result<()> {
    let shadow = unsafe { target.CreateEffect(&CLSID_D2D1Shadow)? };
    let shadow_color = [0.0_f32, 0.0, 0.0, alpha];
    let offset = Vector2 { X: 0.0, Y: offset_y };
    unsafe {
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
        shadow.SetInput(0, mask, true);
        let output = shadow.GetOutput()?;
        target.DrawImage(
            &output,
            Some(&offset),
            None,
            D2D1_INTERPOLATION_MODE_LINEAR,
            D2D1_COMPOSITE_MODE_SOURCE_OVER,
        );
    }
    Ok(())
}

fn draw_surface_backdrop(
    target: &ID2D1DeviceContext,
    background: &ID2D1CommandList,
    surface: &D2D1_ROUNDED_RECT,
    theme: Theme,
) -> Result<()> {
    let blur = unsafe { target.CreateEffect(&CLSID_D2D1GaussianBlur)? };
    let saturation = unsafe { target.CreateEffect(&CLSID_D2D1Saturation)? };
    let sigma = 20.0_f32;
    let saturation_amount = 1.18_f32;
    let factory = unsafe { target.GetFactory()? };
    let geometry: ID2D1Geometry =
        unsafe { factory.CreateRoundedRectangleGeometry(surface)?.cast()? };
    let layer = unsafe { target.CreateLayer(None)? };
    let parameters = D2D1_LAYER_PARAMETERS1 {
        contentBounds: surface.rect,
        geometricMask: ManuallyDrop::new(Some(geometry)),
        maskAntialiasMode: D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
        maskTransform: Matrix3x2 { M11: 1.0, M12: 0.0, M21: 0.0, M22: 1.0, M31: 0.0, M32: 0.0 },
        opacity: if theme.dark { 0.76 } else { 0.66 },
        opacityBrush: ManuallyDrop::new(None),
        layerOptions: D2D1_LAYER_OPTIONS1_NONE,
    };
    unsafe {
        blur.SetValue(
            D2D1_GAUSSIANBLUR_PROP_STANDARD_DEVIATION.0 as u32,
            D2D1_PROPERTY_TYPE_FLOAT,
            bytes_of(&sigma),
        )?;
        blur.SetInput(0, background, true);
        saturation.SetValue(
            D2D1_SATURATION_PROP_SATURATION.0 as u32,
            D2D1_PROPERTY_TYPE_FLOAT,
            bytes_of(&saturation_amount),
        )?;
        let blurred = blur.GetOutput()?;
        saturation.SetInput(0, &blurred, true);
        let output = saturation.GetOutput()?;
        target.PushLayer(&parameters, &layer);
        target.DrawImage(
            &output,
            None,
            None,
            D2D1_INTERPOLATION_MODE_LINEAR,
            D2D1_COMPOSITE_MODE_SOURCE_OVER,
        );
        target.PopLayer();
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn draw_header_content(
    target: &ID2D1DeviceContext,
    dwrite: &IDWriteFactory,
    formats: &FormatSet,
    brushes: &Brushes,
    state: &DisplayState,
    theme: Theme,
    button_state: RefreshButtonState,
    refreshing: bool,
    refresh_angle_degrees: f32,
) -> Result<()> {
    let button_brush = if theme.high_contrast {
        solid_brush(target, refresh_button_fill(theme, button_state))?
    } else {
        let alpha = match (theme.dark, button_state) {
            (false, RefreshButtonState::Idle) => 0.22,
            (false, RefreshButtonState::Hovered) => 0.38,
            (false, RefreshButtonState::Pressed) => 0.46,
            (true, RefreshButtonState::Idle) => 0.055,
            (true, RefreshButtonState::Hovered) => 0.10,
            (true, RefreshButtonState::Pressed) => 0.14,
        };
        solid_brush_alpha(target, refresh_button_fill(theme, button_state), alpha)?
    };
    let button = rounded_rect(370.0, 12.0, 406.0, 48.0, 11.0);
    unsafe {
        if !theme.high_contrast {
            let dot_halo = solid_brush_alpha(target, accent_for(state, theme), 0.10)?;
            target.FillEllipse(&ellipse(22.0, 30.0, 7.0), &dot_halo);
        }
        target.FillEllipse(&ellipse(22.0, 30.0, 3.5), &brushes.accent);
        draw_text(
            target,
            HEADER_BRAND,
            rect(34.0, 12.0, 180.0, 48.0),
            &formats.header,
            &brushes.text,
        );
        draw_tabular_text(
            target,
            dwrite,
            formats,
            &updated_text(state, formats.locale),
            rect(190.0, 12.0, 358.0, 48.0),
            &formats.update,
            &brushes.muted,
        )?;
        target.FillRoundedRectangle(&button, &button_brush);
        if theme.high_contrast {
            target.DrawRoundedRectangle(&button, &brushes.line, 1.0, None::<&ID2D1StrokeStyle>);
        } else {
            let button_line = solid_brush_alpha(
                target,
                rgb(255, 255, 255),
                if theme.dark { 0.10 } else { 0.58 },
            )?;
            target.DrawRoundedRectangle(&button, &button_line, 1.0, None::<&ID2D1StrokeStyle>);
        }
        let icon_brush = if button_state == RefreshButtonState::Pressed {
            &brushes.text
        } else {
            &brushes.muted
        };
        draw_ring_arrow(
            target,
            388.0,
            30.0,
            7.4,
            icon_brush,
            1.7,
            if refreshing { refresh_angle_degrees } else { 0.0 },
        )?;
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
) -> Result<()> {
    unsafe {
        draw_text(
            target,
            locale.text("Weekly remaining", "本周剩余"),
            rect(32.0, 72.0, 204.0, 100.0),
            &formats.label,
            &brushes.muted,
        );
    }
    draw_percentage(target, dwrite, formats, brushes, state.weekly_percent())?;

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
            rect(216.0, 72.0, 388.0, 100.0),
            &formats.label_trailing,
            &brushes.muted,
        );
        draw_tabular_text(
            target,
            dwrite,
            formats,
            &reset.0,
            rect(204.0, 97.0, 388.0, 126.0),
            &formats.reset_value,
            &brushes.text,
        )?;
        draw_tabular_text(
            target,
            dwrite,
            formats,
            &reset.1,
            rect(204.0, 125.0, 388.0, 151.0),
            &formats.secondary,
            &brushes.muted,
        )?;
    }

    draw_quota_track(target, dwrite, formats, brushes, state, locale, theme)?;
    Ok(())
}

fn draw_percentage(
    target: &ID2D1DeviceContext,
    dwrite: &IDWriteFactory,
    formats: &FormatSet,
    brushes: &Brushes,
    percent: Option<u8>,
) -> Result<()> {
    let number = percent.map_or_else(|| "--".to_owned(), |value| value.to_string());
    let layout = percentage_layout(dwrite, formats, &number)?;
    let origin = Vector2 { X: 32.0, Y: 91.0 };
    unsafe {
        target.DrawTextLayout(origin, &layout, &brushes.text, D2D1_DRAW_TEXT_OPTIONS_NONE);
        if percent.is_some() {
            let mut metrics = DWRITE_TEXT_METRICS::default();
            layout.GetMetrics(&mut metrics)?;
            draw_text(
                target,
                "%",
                rect(32.0 + metrics.width + 5.0, 109.0, 176.0, 139.0),
                &formats.quota_suffix,
                &brushes.accent,
            );
        }
    }
    Ok(())
}

fn percentage_layout(
    factory: &IDWriteFactory,
    formats: &FormatSet,
    text: &str,
) -> Result<IDWriteTextLayout> {
    let text_utf16: Vec<u16> = text.encode_utf16().collect();
    let layout = unsafe { factory.CreateTextLayout(&text_utf16, &formats.quota, 176.0, 78.0)? };
    unsafe {
        layout.SetTypography(
            &formats.tabular,
            DWRITE_TEXT_RANGE { startPosition: 0, length: text_utf16.len() as u32 },
        )?;
    }
    Ok(layout)
}

#[allow(clippy::too_many_arguments)]
fn draw_quota_track(
    target: &ID2D1DeviceContext,
    dwrite: &IDWriteFactory,
    formats: &FormatSet,
    brushes: &Brushes,
    state: &DisplayState,
    locale: Locale,
    theme: Theme,
) -> Result<()> {
    let left = 32.0;
    let right = 388.0;
    let top = 176.0;
    let bottom = 184.0;
    let track = if theme.high_contrast {
        brushes.track.clone()
    } else {
        solid_brush_alpha(
            target,
            if theme.dark { rgb(255, 255, 255) } else { rgb(71, 82, 92) },
            if theme.dark { 0.095 } else { 0.13 },
        )?
    };
    unsafe {
        target.FillRoundedRectangle(&rounded_rect(left, top, right, bottom, 4.0), &track);
    }

    if let Some(value) = state.weekly_percent()
        && value > 0
    {
        let filled = (left + (right - left) * f32::from(value) / 100.0).clamp(left + 6.0, right);
        let progress = capacity_gradient(
            target,
            theme,
            value,
            Vector2 { X: left, Y: top },
            Vector2 { X: right, Y: top },
        )?;
        unsafe {
            if theme.high_contrast {
                target.FillRoundedRectangle(
                    &rounded_rect(left, top, filled, bottom, 4.0),
                    &brushes.accent,
                );
            } else {
                target
                    .FillRoundedRectangle(&rounded_rect(left, top, filled, bottom, 4.0), &progress);
            }
            if !theme.high_contrast && filled > left + 12.0 {
                let progress_highlight = solid_brush_alpha(
                    target,
                    rgb(255, 255, 255),
                    if theme.dark { 0.16 } else { 0.18 },
                )?;
                target.DrawLine(
                    Vector2 { X: left + 4.0, Y: top + 1.15 },
                    Vector2 { X: filled - 4.0, Y: top + 1.15 },
                    &progress_highlight,
                    0.65,
                    None::<&ID2D1StrokeStyle>,
                );
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
    let pace_text = pace_label(pace_insight, locale);
    draw_tabular_text(
        target,
        dwrite,
        formats,
        &pace_text,
        rect(32.0, 185.0, 292.0, 214.0),
        &formats.pace,
        &brushes.muted,
    )?;
    if let Some(value) = state.weekly_percent() {
        unsafe {
            draw_text(
                target,
                quota_status_label(value, locale),
                rect(292.0, 185.0, 388.0, 214.0),
                &formats.pace_status,
                &brushes.accent,
            );
        }
    }
    Ok(())
}

fn quota_status_label(value: u8, locale: Locale) -> &'static str {
    if value < 20 {
        locale.text("Quota tight", "额度紧张")
    } else if value < 50 {
        locale.text("Watch usage", "注意用量")
    } else {
        locale.text("Usage healthy", "用量健康")
    }
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
    let credit_expiry = credit_expiry_text(state, locale, Local::now().timestamp());
    let separator = if theme.high_contrast {
        brushes.line.clone()
    } else {
        solid_brush_alpha(
            target,
            if theme.dark { rgb(255, 255, 255) } else { rgb(33, 43, 50) },
            if theme.dark { 0.085 } else { 0.09 },
        )?
    };
    unsafe {
        target.DrawLine(
            Vector2 { X: 32.0, Y: 230.0 },
            Vector2 { X: 388.0, Y: 230.0 },
            &separator,
            1.0,
            None::<&ID2D1StrokeStyle>,
        );
    }

    if let Some(session) = session {
        unsafe {
            for x in [145.0, 275.0] {
                target.DrawLine(
                    Vector2 { X: x, Y: 246.0 },
                    Vector2 { X: x, Y: 338.0 },
                    &separator,
                    1.0,
                    None::<&ID2D1StrokeStyle>,
                );
            }
        }
        draw_metric(
            target,
            dwrite,
            formats,
            brushes,
            rect(14.0, 242.0, 145.0, 350.0),
            locale.text("Plan", "套餐"),
            &plan,
            None,
        )?;
        draw_metric(
            target,
            dwrite,
            formats,
            brushes,
            rect(145.0, 242.0, 275.0, 350.0),
            locale.text("Session quota", "会话额度"),
            &session,
            None,
        )?;
        draw_metric(
            target,
            dwrite,
            formats,
            brushes,
            rect(275.0, 242.0, 406.0, 350.0),
            locale.text("Reset credits", "重置机会"),
            &credits,
            credit_expiry.as_deref(),
        )?;
    } else {
        unsafe {
            target.DrawLine(
                Vector2 { X: 230.0, Y: 246.0 },
                Vector2 { X: 230.0, Y: 338.0 },
                &separator,
                1.0,
                None::<&ID2D1StrokeStyle>,
            );
        }
        draw_metric(
            target,
            dwrite,
            formats,
            brushes,
            rect(14.0, 242.0, 230.0, 350.0),
            locale.text("Plan", "套餐"),
            &plan,
            None,
        )?;
        draw_metric(
            target,
            dwrite,
            formats,
            brushes,
            rect(230.0, 242.0, 406.0, 350.0),
            locale.text("Reset credits", "重置机会"),
            &credits,
            credit_expiry.as_deref(),
        )?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn draw_metric(
    target: &ID2D1DeviceContext,
    dwrite: &IDWriteFactory,
    formats: &FormatSet,
    brushes: &Brushes,
    area: D2D_RECT_F,
    label: &str,
    value: &str,
    secondary: Option<&str>,
) -> Result<()> {
    unsafe {
        draw_text(
            target,
            label,
            rect(area.left + 18.0, area.top + 9.0, area.right - 18.0, area.top + 38.0),
            &formats.metric_label,
            &brushes.muted,
        );
        draw_tabular_text(
            target,
            dwrite,
            formats,
            value,
            rect(area.left + 18.0, area.top + 35.0, area.right - 18.0, area.top + 67.0),
            &formats.metric_value,
            &brushes.text,
        )?;
        if let Some(secondary) = secondary {
            draw_text(
                target,
                secondary,
                rect(area.left + 18.0, area.top + 65.0, area.right - 12.0, area.bottom - 4.0),
                &formats.metric_label,
                &brushes.muted,
            );
        }
    }
    Ok(())
}

fn draw_footer_content(
    target: &ID2D1DeviceContext,
    dwrite: &IDWriteFactory,
    formats: &FormatSet,
    brushes: &Brushes,
    state: &DisplayState,
    locale: Locale,
) -> Result<()> {
    let primary = footer_primary(state, locale, Local::now().timestamp());
    let text = primary.text();
    let text_utf16: Vec<u16> = text.encode_utf16().collect();
    let layout = unsafe { dwrite.CreateTextLayout(&text_utf16, &formats.footer, 286.0, 18.0)? };
    if let Some(duration) = primary.duration.as_deref() {
        let start = primary.prefix.encode_utf16().count() as u32;
        let length = duration.encode_utf16().count() as u32;
        unsafe {
            layout.SetTypography(
                &formats.tabular,
                DWRITE_TEXT_RANGE { startPosition: start, length },
            )?;
            layout.SetDrawingEffect(
                &brushes.accent,
                DWRITE_TEXT_RANGE { startPosition: start, length },
            )?;
        }
    }
    let primary_brush =
        if state.error.is_some() { &brushes.error } else { &brushes.footer_primary };
    unsafe {
        target.FillEllipse(&ellipse(21.0, 396.0, 3.0), primary_brush);
        target.DrawTextLayout(
            Vector2 { X: 30.0, Y: 384.0 },
            &layout,
            primary_brush,
            D2D1_DRAW_TEXT_OPTIONS_NONE,
        );
        draw_text(
            target,
            locale.text("Privacy", "隐私"),
            rect(330.0, 380.0, 402.0, 408.0),
            &formats.update,
            &brushes.muted,
        );
    }
    Ok(())
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

fn draw_tabular_text(
    target: &ID2D1DeviceContext,
    factory: &IDWriteFactory,
    formats: &FormatSet,
    value: &str,
    area: D2D_RECT_F,
    format: &IDWriteTextFormat,
    brush: &ID2D1SolidColorBrush,
) -> Result<()> {
    let text: Vec<u16> = value.encode_utf16().collect();
    let layout = unsafe {
        factory.CreateTextLayout(
            &text,
            format,
            (area.right - area.left).max(0.0),
            (area.bottom - area.top).max(0.0),
        )?
    };
    unsafe {
        layout.SetTypography(
            &formats.tabular,
            DWRITE_TEXT_RANGE { startPosition: 0, length: text.len() as u32 },
        )?;
        target.DrawTextLayout(
            Vector2 { X: area.left, Y: area.top },
            &layout,
            brush,
            D2D1_DRAW_TEXT_OPTIONS_NONE,
        );
    }
    Ok(())
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

fn draw_broad_aurora(target: &ID2D1DeviceContext, theme: Theme) -> Result<()> {
    let alpha = |light: f32, dark: f32| if theme.dark { dark } else { light };
    let pools = [
        radial_gradient(
            target,
            Vector2 { X: -6.0, Y: 6.0 },
            250.0,
            210.0,
            if theme.dark { rgb(39, 180, 190) } else { rgb(76, 207, 226) },
            alpha(0.25, 0.31),
        )?,
        radial_gradient(
            target,
            Vector2 { X: 420.0, Y: 20.0 },
            250.0,
            220.0,
            rgb(230, 100, 157),
            alpha(0.18, 0.22),
        )?,
        radial_gradient(
            target,
            Vector2 { X: 420.0, Y: 410.0 },
            270.0,
            230.0,
            rgb(237, 158, 73),
            alpha(0.13, 0.18),
        )?,
    ];
    unsafe {
        for pool in &pools {
            target.FillRectangle(&rect(0.0, 0.0, 420.0, 412.0), pool);
        }
    }
    Ok(())
}

fn aurora_brushes(
    target: &ID2D1DeviceContext,
    theme: Theme,
) -> Result<[ID2D1RadialGradientBrush; 3]> {
    let alpha = |light: f32, dark: f32| if theme.dark { dark } else { light };
    Ok([
        radial_gradient(
            target,
            Vector2 { X: 32.0, Y: 32.0 },
            130.0,
            92.0,
            rgb(61, 201, 224),
            alpha(0.18, 0.155),
        )?,
        radial_gradient(
            target,
            Vector2 { X: 435.0, Y: 38.0 },
            124.0,
            88.0,
            rgb(238, 113, 163),
            alpha(0.115, 0.09),
        )?,
        radial_gradient(
            target,
            Vector2 { X: 425.0, Y: 411.0 },
            138.0,
            94.0,
            rgb(242, 165, 80),
            alpha(0.075, 0.065),
        )?,
    ])
}

fn draw_aurora_pools(target: &ID2D1DeviceContext, theme: Theme) -> Result<()> {
    let pools = aurora_brushes(target, theme)?;
    unsafe {
        for pool in &pools {
            target.FillRectangle(&rect(0.0, 0.0, 420.0, 412.0), pool);
        }
    }
    Ok(())
}

fn draw_blurred_aurora(target: &ID2D1DeviceContext, theme: Theme) -> Result<()> {
    let pools = aurora_brushes(target, theme)?;
    let blur = unsafe { target.CreateEffect(&CLSID_D2D1GaussianBlur)? };
    let sigma = 24.0_f32;
    unsafe {
        blur.SetValue(
            D2D1_GAUSSIANBLUR_PROP_STANDARD_DEVIATION.0 as u32,
            D2D1_PROPERTY_TYPE_FLOAT,
            bytes_of(&sigma),
        )?;
        let original = target.GetTarget()?;
        let command_list = target.CreateCommandList()?;
        target.SetTarget(&command_list);
        target.Clear(Some(&color_alpha(rgb(0, 0, 0), 0.0)));
        for pool in &pools {
            target.FillRectangle(&rect(0.0, 0.0, 420.0, 412.0), pool);
        }
        target.SetTarget(&original);
        command_list.Close()?;
        blur.SetInput(0, &command_list, true);
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
    rotation_degrees: f32,
) -> Result<()> {
    // A 292° clockwise arc leaves a deliberate 68° gap. The arrowhead is
    // proportional to the radius and filled, so it remains a triangle rather
    // than collapsing into a tiny hooked stroke at high DPI.
    let start_angle = (28.0_f32 + rotation_degrees).to_radians();
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

fn linear_gradient_three(
    target: &ID2D1DeviceContext,
    colors: [(COLORREF, f32); 3],
    start: Vector2,
    end: Vector2,
) -> Result<ID2D1LinearGradientBrush> {
    let stops = [
        D2D1_GRADIENT_STOP { position: 0.0, color: color_alpha(colors[0].0, colors[0].1) },
        D2D1_GRADIENT_STOP { position: 0.52, color: color_alpha(colors[1].0, colors[1].1) },
        D2D1_GRADIENT_STOP { position: 1.0, color: color_alpha(colors[2].0, colors[2].1) },
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

fn radial_gradient(
    target: &ID2D1DeviceContext,
    center: Vector2,
    radius_x: f32,
    radius_y: f32,
    tint: COLORREF,
    alpha: f32,
) -> Result<ID2D1RadialGradientBrush> {
    let stops = [
        D2D1_GRADIENT_STOP { position: 0.0, color: color_alpha(tint, alpha) },
        D2D1_GRADIENT_STOP { position: 0.48, color: color_alpha(tint, alpha * 0.58) },
        D2D1_GRADIENT_STOP { position: 1.0, color: color_alpha(tint, 0.0) },
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
        let properties = D2D1_RADIAL_GRADIENT_BRUSH_PROPERTIES {
            center,
            gradientOriginOffset: Vector2 { X: 0.0, Y: 0.0 },
            radiusX: radius_x,
            radiusY: radius_y,
        };
        target.CreateRadialGradientBrush(&properties, None, &collection)
    }
}

fn capacity_gradient(
    target: &ID2D1DeviceContext,
    theme: Theme,
    value: u8,
    start: Vector2,
    end: Vector2,
) -> Result<ID2D1LinearGradientBrush> {
    let colors = if value < 20 {
        if theme.dark {
            [rgb(235, 140, 149), rgb(226, 116, 127), rgb(215, 95, 106), rgb(208, 106, 115)]
        } else {
            [rgb(234, 124, 134), rgb(218, 99, 111), rgb(198, 78, 91), rgb(185, 75, 85)]
        }
    } else if value < 50 {
        if theme.dark {
            [rgb(239, 189, 104), rgb(224, 166, 75), rgb(208, 146, 55), rgb(197, 138, 50)]
        } else {
            [rgb(240, 180, 90), rgb(219, 146, 57), rgb(191, 121, 30), rgb(169, 107, 22)]
        }
    } else if theme.dark {
        [rgb(75, 210, 191), rgb(37, 189, 157), rgb(16, 165, 129), rgb(11, 140, 112)]
    } else {
        [rgb(67, 205, 184), rgb(34, 181, 150), rgb(16, 163, 127), rgb(11, 144, 115)]
    };
    let stops = [
        D2D1_GRADIENT_STOP { position: 0.0, color: color(colors[0]) },
        D2D1_GRADIENT_STOP { position: 0.38, color: color(colors[1]) },
        D2D1_GRADIENT_STOP { position: 0.70, color: color(colors[2]) },
        D2D1_GRADIENT_STOP { position: 1.0, color: color(colors[3]) },
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
    fn integrated_surface_stays_inside_the_logical_flyout() {
        let surface = rounded_rect(14.0, 58.0, 406.0, 366.0, 16.0);
        assert!(surface.rect.left >= 0.0);
        assert!(surface.rect.top >= 0.0);
        assert!(surface.rect.right <= super::super::CARD_WIDTH as f32);
        assert!(surface.rect.bottom <= super::super::CARD_HEIGHT as f32);
    }
}
