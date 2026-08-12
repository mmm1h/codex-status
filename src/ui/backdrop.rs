//! Window transparency policy for the flyout.
//!
//! The live material is owned by the rounded Windows.UI.Composition tree in
//! `composition.rs`. SWCA Acrylic is deliberately disabled here: it creates a
//! rectangular redirection surface outside that tree and ignores the HWND
//! region, which exposes grey blocks at the four corners.

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
    // Clear every HWND-owned material before the compositor tree is attached.
    // The call also removes stale Acrylic left by an in-place app update.
    clear_official_backdrop(hwnd);
    disable_accent(hwnd);

    let glass_enabled = !theme.high_contrast && transparency_enabled() && !remote_session();
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
