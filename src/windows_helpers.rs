//! Small, event-driven Win32 helpers used by the tray application.
//!
//! The registration types intentionally own their corresponding Win32
//! registration. Keep them alive for as long as the receiving window exists
//! and drop them before destroying that window.

use std::mem::size_of;
use std::ptr;
use std::thread;
use std::time::Duration;

use windows::Win32::Foundation::{
    ERROR_SUCCESS, GetLastError, GlobalFree, HANDLE, HGLOBAL, HWND, SetLastError, WIN32_ERROR,
};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData,
};
use windows::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock};
use windows::Win32::System::RemoteDesktop::{
    NOTIFY_FOR_THIS_SESSION, WTSRegisterSessionNotification, WTSUnRegisterSessionNotification,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    HOT_KEY_MODIFIERS, RegisterHotKey, UnregisterHotKey,
};
use windows::Win32::UI::WindowsAndMessaging::{
    PBT_APMRESUMEAUTOMATIC, PBT_APMRESUMECRITICAL, PBT_APMRESUMESTANDBY, PBT_APMRESUMESUSPEND,
    PBT_APMSTANDBY, PBT_APMSUSPEND, WM_POWERBROADCAST, WM_WTSSESSION_CHANGE, WTS_SESSION_LOCK,
    WTS_SESSION_UNLOCK,
};

// The standard Win32 clipboard format identifier. Defining the value locally
// avoids enabling the much larger Win32_System_Ole projection only for this
// constant.
const CF_UNICODETEXT_FORMAT: u32 = 13;
const CLIPBOARD_OPEN_ATTEMPTS: usize = 5;
const CLIPBOARD_RETRY_DELAY: Duration = Duration::from_millis(25);

/// Errors that can occur while copying text to the Windows clipboard.
#[derive(Debug, thiserror::Error)]
pub enum ClipboardError {
    #[error("the clipboard owner window handle is null")]
    InvalidOwner,
    #[error("clipboard text contains an embedded NUL character")]
    EmbeddedNul,
    #[error("clipboard text is too large")]
    TextTooLarge,
    #[error("Windows clipboard operation failed: {0}")]
    Windows(#[from] windows::core::Error),
}

/// Writes `text` to the system clipboard as `CF_UNICODETEXT`.
///
/// `owner` must be a live window owned by the calling thread. The clipboard is
/// held only while it is emptied and the already-prepared memory block is
/// transferred to Windows. On success Windows owns the allocation; on every
/// failure path this function releases it itself.
pub fn write_unicode_text(owner: HWND, text: &str) -> Result<(), ClipboardError> {
    if owner.0.is_null() {
        return Err(ClipboardError::InvalidOwner);
    }

    let encoded = encode_unicode_text(text)?;
    let byte_len =
        encoded.len().checked_mul(size_of::<u16>()).ok_or(ClipboardError::TextTooLarge)?;

    // Prepare the data before opening the clipboard so other applications are
    // blocked for the shortest practical time.
    let mut memory = OwnedGlobalMemory::allocate(byte_len)?;
    memory.write_utf16(&encoded)?;

    let _clipboard = open_clipboard_with_retry(owner)?;
    unsafe {
        EmptyClipboard()?;
        SetClipboardData(CF_UNICODETEXT_FORMAT, Some(HANDLE(memory.handle().0)))?;
    }

    // SetClipboardData transfers ownership only after it reports success.
    memory.release_to_system();
    Ok(())
}

fn open_clipboard_with_retry(owner: HWND) -> windows::core::Result<OpenClipboardGuard> {
    retry_with_delay(
        || OpenClipboardGuard::open(owner),
        CLIPBOARD_OPEN_ATTEMPTS,
        CLIPBOARD_RETRY_DELAY,
        thread::sleep,
    )
}

fn retry_with_delay<T, E>(
    mut operation: impl FnMut() -> Result<T, E>,
    attempts: usize,
    delay: Duration,
    mut sleep: impl FnMut(Duration),
) -> Result<T, E> {
    assert!(attempts > 0);
    let mut attempted = 1;
    loop {
        match operation() {
            Ok(value) => return Ok(value),
            Err(error) if attempted == attempts => return Err(error),
            Err(_) => {
                sleep(delay);
                attempted += 1;
            }
        }
    }
}

fn encode_unicode_text(text: &str) -> Result<Vec<u16>, ClipboardError> {
    if text.contains('\0') {
        return Err(ClipboardError::EmbeddedNul);
    }

    let mut encoded: Vec<u16> = text.encode_utf16().collect();
    encoded.push(0);
    Ok(encoded)
}

struct OpenClipboardGuard;

impl OpenClipboardGuard {
    fn open(owner: HWND) -> windows::core::Result<Self> {
        unsafe {
            OpenClipboard(Some(owner))?;
        }
        Ok(Self)
    }
}

impl Drop for OpenClipboardGuard {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseClipboard();
        }
    }
}

struct OwnedGlobalMemory {
    handle: Option<HGLOBAL>,
}

impl OwnedGlobalMemory {
    fn allocate(byte_len: usize) -> windows::core::Result<Self> {
        let handle = unsafe { GlobalAlloc(GMEM_MOVEABLE, byte_len)? };
        Ok(Self { handle: Some(handle) })
    }

    fn handle(&self) -> HGLOBAL {
        self.handle.expect("global memory handle must be present")
    }

    fn write_utf16(&mut self, encoded: &[u16]) -> windows::core::Result<()> {
        let handle = self.handle();
        let destination = unsafe { GlobalLock(handle) }.cast::<u16>();
        if destination.is_null() {
            return Err(windows::core::Error::from_thread());
        }

        let mut lock = GlobalMemoryLock::new(handle);
        unsafe {
            ptr::copy_nonoverlapping(encoded.as_ptr(), destination, encoded.len());
        }
        lock.unlock()
    }

    fn release_to_system(&mut self) {
        self.handle = None;
    }
}

impl Drop for OwnedGlobalMemory {
    fn drop(&mut self) {
        if let Some(handle) = self.handle.take() {
            unsafe {
                let _ = GlobalFree(Some(handle));
            }
        }
    }
}

struct GlobalMemoryLock {
    handle: HGLOBAL,
    locked: bool,
}

impl GlobalMemoryLock {
    fn new(handle: HGLOBAL) -> Self {
        Self { handle, locked: true }
    }

    fn unlock(&mut self) -> windows::core::Result<()> {
        // GlobalUnlock returns zero both when the final lock is successfully
        // released and when it fails. ERROR_SUCCESS distinguishes those cases.
        unsafe {
            SetLastError(WIN32_ERROR(0));
            let result = GlobalUnlock(self.handle);
            let last_error = GetLastError();
            if result.is_ok() || last_error == ERROR_SUCCESS {
                self.locked = false;
                Ok(())
            } else {
                Err(windows::core::Error::from_thread())
            }
        }
    }
}

impl Drop for GlobalMemoryLock {
    fn drop(&mut self) {
        if self.locked {
            unsafe {
                let _ = GlobalUnlock(self.handle);
            }
        }
    }
}

/// Owns one `RegisterHotKey` registration and unregisters it on drop.
#[must_use = "dropping the registration immediately unregisters the hotkey"]
#[derive(Debug)]
pub struct HotKeyRegistration {
    hwnd: Option<HWND>,
    id: i32,
    registered: bool,
}

impl HotKeyRegistration {
    /// Registers a window- or thread-associated global hotkey.
    ///
    /// Pass `Some(hwnd)` to receive `WM_HOTKEY` in that window, or `None` to
    /// receive it in the current thread's message queue.
    pub fn register(
        hwnd: Option<HWND>,
        id: i32,
        modifiers: HOT_KEY_MODIFIERS,
        key: u32,
    ) -> windows::core::Result<Self> {
        unsafe {
            RegisterHotKey(hwnd, id, modifiers, key)?;
        }
        Ok(Self { hwnd, id, registered: true })
    }

    /// Unregisters now. On failure the registration remains armed so `Drop`
    /// can make one final best-effort cleanup attempt.
    pub fn unregister(&mut self) -> windows::core::Result<()> {
        if !self.registered {
            return Ok(());
        }

        unsafe {
            UnregisterHotKey(self.hwnd, self.id)?;
        }
        self.registered = false;
        Ok(())
    }
}

impl Drop for HotKeyRegistration {
    fn drop(&mut self) {
        let _ = self.unregister();
    }
}

/// Owns one WTS session-notification registration.
#[must_use = "dropping the registration immediately stops session notifications"]
#[derive(Debug)]
pub struct SessionNotificationRegistration {
    hwnd: HWND,
    registered: bool,
}

impl SessionNotificationRegistration {
    /// Registers lock/unlock notifications for the current session.
    pub fn register(hwnd: HWND) -> windows::core::Result<Self> {
        unsafe {
            WTSRegisterSessionNotification(hwnd, NOTIFY_FOR_THIS_SESSION)?;
        }
        Ok(Self { hwnd, registered: true })
    }

    /// Unregisters now. On failure the registration remains armed so `Drop`
    /// can make one final best-effort cleanup attempt.
    pub fn unregister(&mut self) -> windows::core::Result<()> {
        if !self.registered {
            return Ok(());
        }

        unsafe {
            WTSUnRegisterSessionNotification(self.hwnd)?;
        }
        self.registered = false;
        Ok(())
    }
}

impl Drop for SessionNotificationRegistration {
    fn drop(&mut self) {
        let _ = self.unregister();
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionChangeEvent {
    Locked,
    Unlocked,
}

/// Classifies a window message as a lock or unlock event.
///
/// Pass `message` and `wparam.0` directly from the window procedure.
pub fn session_change_event(message: u32, reason: usize) -> Option<SessionChangeEvent> {
    if message != WM_WTSSESSION_CHANGE {
        return None;
    }

    match u32::try_from(reason).ok()? {
        WTS_SESSION_LOCK => Some(SessionChangeEvent::Locked),
        WTS_SESSION_UNLOCK => Some(SessionChangeEvent::Unlocked),
        _ => None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PowerBroadcastEvent {
    Suspend,
    Resume,
}

/// Classifies `WM_POWERBROADCAST` sleep and resume notifications.
///
/// Automatic and user-visible resume variants are intentionally folded into
/// one event; consumers only need to resume timers and request a refresh once.
pub fn power_broadcast_event(message: u32, event: usize) -> Option<PowerBroadcastEvent> {
    if message != WM_POWERBROADCAST {
        return None;
    }

    match u32::try_from(event).ok()? {
        PBT_APMSUSPEND | PBT_APMSTANDBY => Some(PowerBroadcastEvent::Suspend),
        PBT_APMRESUMEAUTOMATIC
        | PBT_APMRESUMESUSPEND
        | PBT_APMRESUMECRITICAL
        | PBT_APMRESUMESTANDBY => Some(PowerBroadcastEvent::Resume),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unicode_clipboard_encoding_is_nul_terminated() {
        let encoded = encode_unicode_text("额度🙂").unwrap();
        let mut expected: Vec<u16> = "额度🙂".encode_utf16().collect();
        expected.push(0);
        assert_eq!(encoded, expected);
    }

    #[test]
    fn empty_clipboard_text_is_a_single_terminator() {
        assert_eq!(encode_unicode_text("").unwrap(), vec![0]);
    }

    #[test]
    fn embedded_nul_is_rejected_instead_of_silently_truncating() {
        assert!(matches!(encode_unicode_text("before\0after"), Err(ClipboardError::EmbeddedNul)));
    }

    #[test]
    fn clipboard_retry_is_bounded_and_stops_after_success() {
        let mut calls = 0;
        let mut delays = Vec::new();
        let result = retry_with_delay(
            || {
                calls += 1;
                if calls < 3 { Err("busy") } else { Ok("copied") }
            },
            CLIPBOARD_OPEN_ATTEMPTS,
            CLIPBOARD_RETRY_DELAY,
            |delay| delays.push(delay),
        );
        assert_eq!(result, Ok("copied"));
        assert_eq!(calls, 3);
        assert_eq!(delays, vec![CLIPBOARD_RETRY_DELAY; 2]);
    }

    #[test]
    fn clipboard_retry_returns_the_last_error_after_the_limit() {
        let mut calls = 0;
        let result = retry_with_delay(
            || {
                calls += 1;
                Err::<(), _>(calls)
            },
            CLIPBOARD_OPEN_ATTEMPTS,
            CLIPBOARD_RETRY_DELAY,
            |_| {},
        );
        assert_eq!(result, Err(CLIPBOARD_OPEN_ATTEMPTS));
        assert_eq!(calls, CLIPBOARD_OPEN_ATTEMPTS);
    }

    #[test]
    fn session_messages_are_classified_strictly() {
        assert_eq!(
            session_change_event(WM_WTSSESSION_CHANGE, WTS_SESSION_LOCK as usize),
            Some(SessionChangeEvent::Locked)
        );
        assert_eq!(
            session_change_event(WM_WTSSESSION_CHANGE, WTS_SESSION_UNLOCK as usize),
            Some(SessionChangeEvent::Unlocked)
        );
        assert_eq!(session_change_event(WM_WTSSESSION_CHANGE, 1), None);
        assert_eq!(session_change_event(WM_POWERBROADCAST, 7), None);
    }

    #[test]
    fn power_messages_cover_modern_and_legacy_resume_variants() {
        for event in [PBT_APMSUSPEND, PBT_APMSTANDBY] {
            assert_eq!(
                power_broadcast_event(WM_POWERBROADCAST, event as usize),
                Some(PowerBroadcastEvent::Suspend)
            );
        }

        for event in [
            PBT_APMRESUMEAUTOMATIC,
            PBT_APMRESUMESUSPEND,
            PBT_APMRESUMECRITICAL,
            PBT_APMRESUMESTANDBY,
        ] {
            assert_eq!(
                power_broadcast_event(WM_POWERBROADCAST, event as usize),
                Some(PowerBroadcastEvent::Resume)
            );
        }

        assert_eq!(power_broadcast_event(WM_POWERBROADCAST, 0), None);
        assert_eq!(power_broadcast_event(WM_WTSSESSION_CHANGE, 4), None);
    }
}
