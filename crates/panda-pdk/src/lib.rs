//! Panda guest-side PDK (Plugin Development Kit).
//!
//! This crate is intended for `wasm32-unknown-unknown` guest plugins.

pub const PANDA_WASM_ABI_VERSION: i32 = 1;
pub const RC_ALLOW: i32 = 0;
pub const RC_REJECT_POLICY_DENIED: i32 = 1;
pub const RC_REJECT_MALFORMED_REQUEST: i32 = 2;

#[cfg(target_arch = "wasm32")]
unsafe extern "C" {
    fn panda_set_header(name_ptr: i32, name_len: i32, value_ptr: i32, value_len: i32);
    fn panda_set_body(ptr: i32, len: i32);
    fn panda_set_response_chunk(ptr: i32, len: i32);
}

/// Host callback: append request header.
#[cfg(target_arch = "wasm32")]
pub fn set_header(name: &[u8], value: &[u8]) {
    unsafe {
        panda_set_header(
            name.as_ptr() as i32,
            name.len() as i32,
            value.as_ptr() as i32,
            value.len() as i32,
        );
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub fn set_header(_name: &[u8], _value: &[u8]) {}

/// Host callback: replace request body.
#[cfg(target_arch = "wasm32")]
pub fn set_body(body: &[u8]) {
    unsafe {
        panda_set_body(body.as_ptr() as i32, body.len() as i32);
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub fn set_body(_body: &[u8]) {}

/// Host callback: replace current response chunk.
#[cfg(target_arch = "wasm32")]
pub fn set_response_chunk(chunk: &[u8]) {
    unsafe {
        panda_set_response_chunk(chunk.as_ptr() as i32, chunk.len() as i32);
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub fn set_response_chunk(_chunk: &[u8]) {}
