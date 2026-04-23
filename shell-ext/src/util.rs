//! Small helpers for bridging Rust strings into the COM ABI. All strings
//! in `IExplorerCommand` crossed as `PWSTR` are allocated with `CoTaskMemAlloc`
//! and freed by the caller (Explorer). The helper below centralises that.

use windows::core::*;
use windows::Win32::System::Com::CoTaskMemAlloc;

/// Allocate a null-terminated UTF-16 copy of `s` on the COM task heap
/// and hand the pointer back as a `PWSTR`. Ownership transfers to
/// whichever COM caller received it.
pub fn cotaskmem_wstr(s: &str) -> PWSTR {
    let mut wide: Vec<u16> = s.encode_utf16().collect();
    wide.push(0);
    let bytes = wide.len() * std::mem::size_of::<u16>();
    unsafe {
        let buf = CoTaskMemAlloc(bytes) as *mut u16;
        if buf.is_null() {
            return PWSTR::null();
        }
        std::ptr::copy_nonoverlapping(wide.as_ptr(), buf, wide.len());
        PWSTR(buf)
    }
}
