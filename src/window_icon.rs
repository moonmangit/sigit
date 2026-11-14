//! Windows: give sigit's window its icon. The taskbar button, Alt+Tab and the window's own
//! menu show the window's icon, not the .exe's, and Rune doesn't set one. So once the window
//! exists, load the icon build.rs embedded in sigit.exe (resource 1) and hand it to the window
//! with WM_SETICON. Elsewhere there's nothing to do: macOS takes the icon from the .app bundle.

/// Set the icon on this thread's windows. True once done (or when there's nothing to do), so
/// the caller can stop asking; false while the window doesn't exist yet.
#[cfg(windows)]
pub fn apply() -> bool {
    use windows_sys::Win32::Foundation::{HWND, LPARAM};
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::System::Threading::GetCurrentThreadId;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        EnumThreadWindows, GetSystemMetrics, ICON_BIG, ICON_SMALL, IMAGE_ICON, LR_DEFAULTCOLOR, LoadImageW, SM_CXICON, SM_CXSMICON, SM_CYICON, SM_CYSMICON, SendMessageW,
        WM_SETICON,
    };
    use windows_sys::core::BOOL;

    struct Icons {
        big: isize,
        small: isize,
        windows: usize,
    }

    unsafe extern "system" fn each(hwnd: HWND, lp: LPARAM) -> BOOL {
        // SAFETY: `lp` is the `Icons` passed below, alive for the whole enumeration.
        let icons = unsafe { &mut *(lp as *mut Icons) };
        unsafe {
            SendMessageW(hwnd, WM_SETICON, ICON_BIG as usize, icons.big);
            SendMessageW(hwnd, WM_SETICON, ICON_SMALL as usize, icons.small);
        }
        icons.windows += 1;
        1 // keep going
    }

    // SAFETY: plain Win32 calls; the icon handles are shared resources owned by the module.
    unsafe {
        let module = GetModuleHandleW(std::ptr::null());
        // MAKEINTRESOURCE(1): the app icon winresource embeds.
        let load = |w, h| LoadImageW(module, 1usize as *const u16, IMAGE_ICON, GetSystemMetrics(w), GetSystemMetrics(h), LR_DEFAULTCOLOR) as isize;
        let big = load(SM_CXICON, SM_CYICON);
        if big == 0 {
            return true; // built without the icon resource: nothing to set
        }
        let small = load(SM_CXSMICON, SM_CYSMICON);
        let mut icons = Icons { big, small: if small == 0 { big } else { small }, windows: 0 };
        EnumThreadWindows(GetCurrentThreadId(), Some(each), &mut icons as *mut Icons as LPARAM);
        icons.windows > 0
    }
}

#[cfg(not(windows))]
pub fn apply() -> bool {
    true
}
