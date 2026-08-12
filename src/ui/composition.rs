//! Rounded desktop composition tree for the flyout.
//!
//! SWCA Acrylic creates a separate rectangular redirection surface outside the
//! compositor tree. The compositor-owned backdrop and swap-chain visuals below
//! instead share one antialiased rounded root clip, which is also the normal
//! path's only visible silhouette. A binary window region is reserved for GDI.

use std::sync::Mutex;
use windows::Foundation::{IPropertyValue, PropertyValue};
use windows::Graphics::Effects::{
    IGraphicsEffect, IGraphicsEffect_Impl, IGraphicsEffectSource, IGraphicsEffectSource_Impl,
};
use windows::System::DispatcherQueueController;
use windows::UI::Composition::Desktop::DesktopWindowTarget;
use windows::UI::Composition::{
    CompositionBackdropBrush, CompositionEffectBrush, CompositionEffectSourceParameter,
    CompositionSurfaceBrush, Compositor, ContainerVisual, ICompositionSurface, SpriteVisual,
};
use windows::Win32::Foundation::{E_INVALIDARG, HWND};
use windows::Win32::Graphics::Direct2D::{
    CLSID_D2D1GaussianBlur, D2D1_GAUSSIANBLUR_OPTIMIZATION_BALANCED,
};
use windows::Win32::Graphics::Dxgi::IDXGISwapChain1;
use windows::Win32::System::WinRT::Composition::{ICompositorDesktopInterop, ICompositorInterop};
use windows::Win32::System::WinRT::Graphics::Direct2D::{
    GRAPHICS_EFFECT_PROPERTY_MAPPING, IGraphicsEffectD2D1Interop, IGraphicsEffectD2D1Interop_Impl,
};
use windows::Win32::System::WinRT::{
    CreateDispatcherQueueController, DQTAT_COM_STA, DQTYPE_THREAD_CURRENT, DispatcherQueueOptions,
    RO_INIT_SINGLETHREADED, RoInitialize, RoUninitialize,
};
use windows::core::{ComObject, Error, GUID, HSTRING, Interface, PCWSTR, Result};
use windows_numerics::Vector2;

const BLUR_STANDARD_DEVIATION: f32 = 18.0;
const FLYOUT_RADIUS: f32 = 20.0;

pub(super) struct WinRtApartment {
    _dispatcher: DispatcherQueueController,
}

impl WinRtApartment {
    pub(super) fn initialize() -> Result<Self> {
        unsafe { RoInitialize(RO_INIT_SINGLETHREADED)? };
        let options = DispatcherQueueOptions {
            dwSize: std::mem::size_of::<DispatcherQueueOptions>() as u32,
            threadType: DQTYPE_THREAD_CURRENT,
            apartmentType: DQTAT_COM_STA,
        };
        let dispatcher = match unsafe { CreateDispatcherQueueController(options) } {
            Ok(dispatcher) => dispatcher,
            Err(error) => {
                unsafe { RoUninitialize() };
                return Err(error);
            }
        };
        Ok(Self { _dispatcher: dispatcher })
    }
}

impl Drop for WinRtApartment {
    fn drop(&mut self) {
        unsafe { RoUninitialize() };
    }
}

pub(super) struct DesktopComposition {
    target: DesktopWindowTarget,
    compositor: Compositor,
    root: ContainerVisual,
    backdrop_visual: SpriteVisual,
    content_visual: SpriteVisual,
    _backdrop: CompositionBackdropBrush,
    _effect_brush: CompositionEffectBrush,
    _surface: ICompositionSurface,
    _surface_brush: CompositionSurfaceBrush,
}

impl DesktopComposition {
    pub(super) fn new(
        hwnd: HWND,
        swap_chain: &IDXGISwapChain1,
        width: u32,
        height: u32,
        dpi: u32,
    ) -> Result<Self> {
        let compositor = stage("create compositor", Compositor::new())?;
        let desktop_interop: ICompositorDesktopInterop =
            stage("cast desktop interop", compositor.cast())?;
        let target = stage("create desktop target", unsafe {
            desktop_interop.CreateDesktopWindowTarget(hwnd, true)
        })?;

        let root = stage("create root visual", compositor.CreateContainerVisual())?;
        let backdrop_visual = stage("create backdrop visual", compositor.CreateSpriteVisual())?;
        let content_visual = stage("create content visual", compositor.CreateSpriteVisual())?;

        let backdrop = stage("create backdrop brush", compositor.CreateBackdropBrush())?;
        let effect_brush = stage("create blur brush", create_blur_brush(&compositor, &backdrop))?;
        stage("set backdrop brush", backdrop_visual.SetBrush(&effect_brush))?;

        let compositor_interop: ICompositorInterop =
            stage("cast compositor interop", compositor.cast())?;
        let surface = stage("wrap swap chain", unsafe {
            compositor_interop.CreateCompositionSurfaceForSwapChain(swap_chain)
        })?;
        let surface_brush =
            stage("create surface brush", compositor.CreateSurfaceBrushWithSurface(&surface))?;
        stage("set content brush", content_visual.SetBrush(&surface_brush))?;

        let children = stage("get root children", root.Children())?;
        stage("insert backdrop", children.InsertAtBottom(&backdrop_visual))?;
        stage("insert content", children.InsertAtTop(&content_visual))?;

        let tree = Self {
            target,
            compositor,
            root,
            backdrop_visual,
            content_visual,
            _backdrop: backdrop,
            _effect_brush: effect_brush,
            _surface: surface,
            _surface_brush: surface_brush,
        };
        tree.resize(width, height, dpi)?;
        // Attach only after the exact rounded geometry exists. Any earlier
        // failure then drops an unattached tree and lets GDI take over cleanly.
        stage("attach root", tree.target.SetRoot(&tree.root))?;
        Ok(tree)
    }

    pub(super) fn resize(&self, width: u32, height: u32, dpi: u32) -> Result<()> {
        let size = Vector2 { X: width as f32, Y: height as f32 };
        self.root.SetSize(size)?;
        self.backdrop_visual.SetSize(size)?;
        self.content_visual.SetSize(size)?;

        let scaled_radius = FLYOUT_RADIUS * dpi.max(96) as f32 / 96.0;
        let radius = Vector2 { X: scaled_radius, Y: scaled_radius };
        let clip = self.compositor.CreateRectangleClipWithSidesAndRadius(
            0.0,
            0.0,
            width as f32,
            height as f32,
            radius,
            radius,
            radius,
            radius,
        )?;
        self.root.SetClip(&clip)
    }

    pub(super) fn set_glass_enabled(&self, enabled: bool) -> Result<()> {
        self.backdrop_visual.SetIsVisible(enabled)
    }

    pub(super) fn detach(self) -> Result<()> {
        self.target.SetRoot(None::<&windows::UI::Composition::Visual>)?;
        self.compositor.Close()
    }
}

fn stage<T>(name: &str, result: Result<T>) -> Result<T> {
    #[cfg(feature = "diagnostics")]
    if let Err(error) = &result {
        eprintln!("Composition {name} failed: {:#x}", error.code().0);
    }
    #[cfg(not(feature = "diagnostics"))]
    let _ = name;
    result
}

fn create_blur_brush(
    compositor: &Compositor,
    backdrop: &CompositionBackdropBrush,
) -> Result<CompositionEffectBrush> {
    let parameter_name = HSTRING::from("Backdrop");
    let source_parameter = CompositionEffectSourceParameter::Create(&parameter_name)?;
    let source: IGraphicsEffectSource = source_parameter.cast()?;
    let effect: IGraphicsEffect = ComObject::new(GaussianBlurEffect {
        name: Mutex::new(HSTRING::from("FlyoutBackdropBlur")),
        source,
    })
    .into_interface();
    let factory = compositor.CreateEffectFactory(&effect)?;
    let brush = factory.CreateBrush()?;
    brush.SetSourceParameter(&parameter_name, backdrop)?;
    Ok(brush)
}

#[windows::core::implement(IGraphicsEffect, IGraphicsEffectSource, IGraphicsEffectD2D1Interop)]
struct GaussianBlurEffect {
    name: Mutex<HSTRING>,
    source: IGraphicsEffectSource,
}

impl IGraphicsEffectSource_Impl for GaussianBlurEffect_Impl {}

impl IGraphicsEffect_Impl for GaussianBlurEffect_Impl {
    fn Name(&self) -> Result<HSTRING> {
        Ok(self.name.lock().expect("effect name lock poisoned").clone())
    }

    fn SetName(&self, name: &HSTRING) -> Result<()> {
        *self.name.lock().expect("effect name lock poisoned") = name.clone();
        Ok(())
    }
}

impl IGraphicsEffectD2D1Interop_Impl for GaussianBlurEffect_Impl {
    fn GetEffectId(&self) -> Result<GUID> {
        Ok(CLSID_D2D1GaussianBlur)
    }

    fn GetNamedPropertyMapping(
        &self,
        _name: &PCWSTR,
        _index: *mut u32,
        _mapping: *mut GRAPHICS_EFFECT_PROPERTY_MAPPING,
    ) -> Result<()> {
        Err(Error::from(E_INVALIDARG))
    }

    fn GetPropertyCount(&self) -> Result<u32> {
        Ok(3)
    }

    fn GetProperty(&self, index: u32) -> Result<IPropertyValue> {
        let value = match index {
            0 => PropertyValue::CreateSingle(BLUR_STANDARD_DEVIATION)?,
            1 => PropertyValue::CreateUInt32(D2D1_GAUSSIANBLUR_OPTIMIZATION_BALANCED.0 as u32)?,
            2 => PropertyValue::CreateUInt32(1)?, // D2D1_BORDER_MODE_HARD
            _ => return Err(Error::from(E_INVALIDARG)),
        };
        value.cast()
    }

    fn GetSource(&self, index: u32) -> Result<IGraphicsEffectSource> {
        if index == 0 { Ok(self.source.clone()) } else { Err(Error::from(E_INVALIDARG)) }
    }

    fn GetSourceCount(&self) -> Result<u32> {
        Ok(1)
    }
}
