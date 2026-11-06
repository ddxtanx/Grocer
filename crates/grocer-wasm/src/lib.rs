//! Browser-side parser: the *same* grocer-parser crate compiled to WASM.
//!
//! Deliberately wasm-bindgen-free — a plain C ABI the JS glue in static/app.js
//! calls directly (alloc / parse / free_str), so the artifact loads with
//! `WebAssembly.instantiateStreaming` and no generated shim layer.
//!
//! Contract:
//!   * alloc(n)              -> byte pointer for n bytes (JS writes OCR text here)
//!   * parse(ptr, len)       -> NUL-terminated UTF-8 JSON (ParsedReceipt)
//!   * free_str(ptr)         -> release a parse() result
//! Memory grows; nothing else crosses the boundary. No network, no storage:
//! the receipt image never leaves the device (only its OCR text reaches this
//! function, and only structured JSON ever leaves the page).

use std::ffi::CString;
use std::os::raw::c_char;

#[no_mangle]
pub extern "C" fn alloc(size: usize) -> *mut u8 {
    let mut v: Vec<u8> = Vec::with_capacity(size + 1);
    v.push(0);
    let p = v.as_mut_ptr();
    std::mem::forget(v);
    p
}

#[no_mangle]
pub unsafe extern "C" fn parse(ptr: *const u8, len: usize) -> *mut c_char {
    if ptr.is_null() {
        return null_json();
    }
    let bytes = std::slice::from_raw_parts(ptr, len);
    // NULs and invalid UTF-8 from OCR are lossily sanitized, never fatal.
    let text = String::from_utf8_lossy(bytes).replace('\0', " ");
    let receipt = grocer_parser::parse(&text);
    let json = match serde_json::to_string(&receipt) {
        Ok(j) => j,
        Err(_) => return null_json(),
    };
    CString::new(json)
        .unwrap_or_else(|_| CString::new("{}").unwrap())
        .into_raw()
}

/// Convenience for the client: chain detection only, no full parse.
/// Returns "fresh_thyme" | "trader_joes" | "costco" | "unknown".
#[no_mangle]
pub unsafe extern "C" fn detect_chain(ptr: *const u8, len: usize) -> *mut c_char {
    let chain = if ptr.is_null() {
        grocer_parser::Chain::Unknown
    } else {
        let bytes = std::slice::from_raw_parts(ptr, len);
        let text = String::from_utf8_lossy(bytes).replace('\0', " ");
        grocer_parser::parse(&text).chain
    };
    let name = match chain {
        grocer_parser::Chain::FreshThyme => "fresh_thyme",
        grocer_parser::Chain::TraderJoes => "trader_joes",
        grocer_parser::Chain::Costco => "costco",
        grocer_parser::Chain::Unknown => "unknown",
    };
    CString::new(name).unwrap().into_raw()
}

fn null_json() -> *mut c_char {
    CString::new("{}").unwrap().into_raw()
}

#[no_mangle]
pub unsafe extern "C" fn free_str(ptr: *mut c_char) {
    if !ptr.is_null() {
        drop(CString::from_raw(ptr));
    }
}
