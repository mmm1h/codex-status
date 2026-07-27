//! Window backdrop policy for the flyout.
//!
//! The acrylic path intentionally uses an undocumented Windows API. Every
//! failure is treated as an expected capability failure and falls back to the
//! existing opaque renderer.
//!
//! The documented `DWMWA_SYSTEMBACKDROP_TYPE` attribute is deliberately *not*
//! used to produce the material, even though this flyout meets its documented
//! requirements. A 41-case ablation on Windows 11 24H2 (build 26100) measured
//! both `DWMSBT_TRANSIENTWINDOW` and `DWMSBT_MAINWINDOW` against a moving
//! high-contrast backdrop: every window-style variant returned `S_OK` and then
//! painted a constant flat colour, with a frame-to-frame pixel delta of exactly
//! zero. Swapping only the backdrop mechanism to `SetWindowCompositionAttribute`
//! produced genuine real-time sampling on the unchanged window. Style factors
//! were ruled out individually — dropping `WS_EX_TOOLWINDOW`, `WS_EX_TOPMOST`,
//! the owner, or `CS_DROPSHADOW`, adding `WS_EX_NOREDIRECTIONBITMAP`, keeping a
//! real non-client frame via `WS_CAPTION | WS_THICKFRAME` plus `WM_NCCALCSIZE`,
//! and bypassing DirectComposition all left the result unchanged.
//!
//! `AccentFlags` matters more than the tint: the value `2` that
//! window-vibrancy passes for plain blur rendered nearly black here, so this
//! module pins `0`. That same version split is why the crate is not used
//! directly — on Windows 11 it routes to the documented attribute that fails
//! above, so its Windows 10 branch would be the only reachable benefit.
//!
//! Re-run the ablation before switching to the documented attribute; do not
//! assume a later Windows build fixed it.

use super::Theme;
use std::cell::Cell;
use std::ffi::{CString, c_void};
use std::mem::{size_of, size_of_val};
use std::sync::OnceLock;
use windows::Win32::Foundation::{FARPROC, HWND};
use windows::Win32::Graphics::Dwm::{
    DWMSBT_NONE, DWMWA_SYSTEMBACKDROP_TYPE, DwmSetWindowAttribute,
};
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_REMOTESESSION};
use windows::core::{BOOL, PCSTR, w};
use winreg::RegKey;
use winreg::enums::HKEY_CURRENT_USER;

const PERSONALIZE_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize";

// These structures and constants are an undocumented Windows ABI. They mirror
// window-vibrancy 0.8.0/src/windows.rs, whose implementation dynamically
// resolves user32!SetWindowCompositionAttribute. Windows updates may change or
// remove this behavior.
const WCA_ACCENT_POLICY: u32 = 19;
const ACCENT_DISABLED: u32 = 0;
const ACCENT_ENABLE_ACRYLICBLURBEHIND: u32 = 4;

// Verified on Windows 11 24H2. GradientColor is packed as
// R | G << 8 | B << 16 | A << 24.
const DARK_ACRYLIC_TINT: u32 = 0x961C_1812;
const LIGHT_ACRYLIC_TINT: u32 = pack_gradient(248, 250, 246, 118);

#[repr(C)]
struct AccentPolicy {
    accent_state: u32,
    accent_flags: u32,
    gradient_color: u32,
    animation_id: u32,
}

#[repr(C)]
struct WindowCompositionAttributeData {
    attribute: u32,
    data: *mut c_void,
    size: usize,
}

type SetWindowCompositionAttribute =
    unsafe extern "system" fn(HWND, *mut WindowCompositionAttributeData) -> BOOL;

#[derive(Clone, Copy)]
struct BackdropState {
    hwnd: HWND,
    glass_enabled: bool,
}

thread_local! {
    static STATE: Cell<Option<BackdropState>> = const { Cell::new(None) };
}

static SWCA: OnceLock<Option<SetWindowCompositionAttribute>> = OnceLock::new();

pub(super) fn configure(hwnd: HWND, theme: Theme) -> bool {
    // Clearing the official system backdrop is required before SWCA acrylic;
    // it also prevents a stale DWM material from fighting the opaque fallback.
    clear_official_backdrop(hwnd);

    let policy_allows_glass = !theme.high_contrast && transparency_enabled() && !remote_session();
    let glass_enabled = if policy_allows_glass {
        apply_accent(
            hwnd,
            ACCENT_ENABLE_ACRYLICBLURBEHIND,
            if theme.dark { DARK_ACRYLIC_TINT } else { LIGHT_ACRYLIC_TINT },
        )
    } else {
        false
    };

    // A failed resolve/call is deliberately non-fatal: remove any prior accent
    // when possible and let Direct2D/GDI paint the current opaque background.
    if !glass_enabled {
        disable_accent(hwnd);
    }
    STATE.with(|state| state.set(Some(BackdropState { hwnd, glass_enabled })));
    glass_enabled
}

pub(super) fn glass_enabled(hwnd: HWND) -> bool {
    STATE.with(|state| state.get().is_some_and(|state| state.hwnd == hwnd && state.glass_enabled))
}

pub(super) fn disable_for_render_fallback(hwnd: HWND) {
    clear_official_backdrop(hwnd);
    disable_accent(hwnd);
    STATE.with(|state| state.set(Some(BackdropState { hwnd, glass_enabled: false })));
}

fn transparency_enabled() -> bool {
    RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey(PERSONALIZE_KEY)
        .ok()
        .and_then(|key| key.get_value::<u32, _>("EnableTransparency").ok())
        // Missing policy values mean "use the Windows default", which is on.
        .is_none_or(|value| value != 0)
}

fn remote_session() -> bool {
    unsafe { GetSystemMetrics(SM_REMOTESESSION) != 0 }
}

fn clear_official_backdrop(hwnd: HWND) {
    let none = DWMSBT_NONE;
    unsafe {
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_SYSTEMBACKDROP_TYPE,
            (&none as *const windows::Win32::Graphics::Dwm::DWM_SYSTEMBACKDROP_TYPE).cast(),
            size_of_val(&none) as u32,
        );
    }
}

fn disable_accent(hwnd: HWND) {
    let _ = apply_accent(hwnd, ACCENT_DISABLED, 0);
}

fn apply_accent(hwnd: HWND, state: u32, gradient_color: u32) -> bool {
    let Some(swca) = resolve_swca() else {
        return false;
    };
    let mut policy = AccentPolicy {
        accent_state: state,
        // Verified critical value: flags other than zero can make state 4
        // nearly black on Windows 11 24H2.
        accent_flags: 0,
        gradient_color,
        animation_id: 0,
    };
    let mut data = WindowCompositionAttributeData {
        attribute: WCA_ACCENT_POLICY,
        data: (&mut policy as *mut AccentPolicy).cast(),
        size: size_of::<AccentPolicy>(),
    };
    unsafe { swca(hwnd, &mut data) }.as_bool()
}

fn resolve_swca() -> Option<SetWindowCompositionAttribute> {
    *SWCA.get_or_init(|| unsafe {
        // Keep user32 loaded for the process lifetime because the cached
        // function pointer must remain valid.
        let module = LoadLibraryW(w!("user32.dll")).ok()?;
        let name = CString::new("SetWindowCompositionAttribute").ok()?;
        let proc: FARPROC = GetProcAddress(module, PCSTR(name.as_ptr().cast()));
        proc.map(|value| std::mem::transmute(value))
    })
}

const fn pack_gradient(r: u8, g: u8, b: u8, a: u8) -> u32 {
    (r as u32) | ((g as u32) << 8) | ((b as u32) << 16) | ((a as u32) << 24)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gradient_color_uses_verified_channel_order() {
        assert_eq!(pack_gradient(18, 24, 28, 150), DARK_ACRYLIC_TINT);
    }
}
