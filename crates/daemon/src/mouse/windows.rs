//! Windows backend: a low-level mouse hook for the X buttons.

use std::sync::{Arc, OnceLock};

use windows_sys::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, GetMessageW, SetWindowsHookExW, MSG, MSLLHOOKSTRUCT, WH_MOUSE_LL,
    WM_XBUTTONDOWN, WM_XBUTTONUP,
};

use super::Shared;

static HOOK_STATE: OnceLock<Arc<Shared>> = OnceLock::new();

/// XBUTTON1 → Mouse4, XBUTTON2 → Mouse5.
fn button_of(info: &MSLLHOOKSTRUCT) -> u8 {
    match (info.mouseData >> 16) & 0xFFFF {
        1 => 4,
        2 => 5,
        _ => 0,
    }
}

unsafe extern "system" fn hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code >= 0 {
        if let Some(shared) = HOOK_STATE.get() {
            let msg = wparam as u32;
            if msg == WM_XBUTTONDOWN || msg == WM_XBUTTONUP {
                // SAFETY: for WH_MOUSE_LL, lparam points to an MSLLHOOKSTRUCT.
                let info = unsafe { &*(lparam as *const MSLLHOOKSTRUCT) };
                let button = button_of(info);
                if shared.wants(button) {
                    if msg == WM_XBUTTONDOWN {
                        shared.pressed(button);
                    }
                    return 1; // consume the event
                }
            }
        }
    }
    // SAFETY: forwarding to the next hook with the original arguments.
    unsafe { CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam) }
}

pub fn start(shared: Arc<Shared>) -> Result<(), String> {
    let _ = HOOK_STATE.set(shared);
    std::thread::Builder::new()
        .name("aw-manual-mouse".into())
        .spawn(|| {
            // SAFETY: standard hook installation and message loop on this thread.
            unsafe {
                let module = GetModuleHandleW(std::ptr::null());
                let h = SetWindowsHookExW(WH_MOUSE_LL, Some(hook), module, 0);
                if h.is_null() {
                    tracing::error!("cannot install the mouse hook");
                    return;
                }
                let mut msg: MSG = std::mem::zeroed();
                while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {}
            }
        })
        .map_err(|e| e.to_string())?;
    Ok(())
}
