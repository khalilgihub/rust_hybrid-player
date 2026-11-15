use windows_sys::Win32::Foundation::{HANDLE, HGLOBAL};
use windows_sys::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, GetClipboardData, OpenClipboard, SetClipboardData,
};
use windows_sys::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};

const CF_UNICODETEXT: u32 = 13;

/// Reads UTF-16 Unicode text from the Win32 clipboard.
pub fn get_clipboard_text() -> Option<String> {
    unsafe {
        if OpenClipboard(std::ptr::null_mut()) == 0 {
            return None;
        }
        let handle = GetClipboardData(CF_UNICODETEXT);
        let result = if handle.is_null() {
            None
        } else {
            let ptr = GlobalLock(handle as HGLOBAL) as *const u16;
            if ptr.is_null() {
                None
            } else {
                let mut len = 0usize;
                while *ptr.add(len) != 0 {
                    len += 1;
                }
                let slice = std::slice::from_raw_parts(ptr, len);
                let text = String::from_utf16_lossy(slice);
                GlobalUnlock(handle as HGLOBAL);
                Some(text)
            }
        };
        CloseClipboard();
        result
    }
}

/// Writes UTF-16 Unicode text to the Win32 clipboard.
pub fn set_clipboard_text(text: &str) -> bool {
    let utf16: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
    let byte_len = utf16.len() * std::mem::size_of::<u16>();
    unsafe {
        if OpenClipboard(std::ptr::null_mut()) == 0 {
            return false;
        }
        EmptyClipboard();
        let hmem = GlobalAlloc(GMEM_MOVEABLE, byte_len);
        if hmem.is_null() {
            CloseClipboard();
            return false;
        }
        let dst = GlobalLock(hmem) as *mut u16;
        if dst.is_null() {
            CloseClipboard();
            return false;
        }
        std::ptr::copy_nonoverlapping(utf16.as_ptr(), dst, utf16.len());
        GlobalUnlock(hmem);
        let set_res: HANDLE = SetClipboardData(CF_UNICODETEXT, hmem as HANDLE);
        CloseClipboard();
        !set_res.is_null()
    }
}
