//! Minimal guest module for Panda ABI v0.
//! Build: `cargo build -p wasm-plugin-sample --target wasm32-unknown-unknown --release`

extern "C" {
    fn panda_set_header(name_ptr: i32, name_len: i32, value_ptr: i32, value_len: i32);
    fn panda_set_body(ptr: i32, len: i32);
}

/// Must match `panda_wasm::PANDA_WASM_ABI_VERSION` (duplicated to avoid a host dep in the guest).
#[no_mangle]
pub extern "C" fn panda_abi_version() -> i32 {
    0
}

/// Optional smoke export called by host startup.
#[no_mangle]
pub extern "C" fn add(a: i32, b: i32) -> i32 {
    a.saturating_add(b)
}

/// Request hook: inject a marker header via host import.
#[no_mangle]
pub extern "C" fn panda_on_request() -> i32 {
    let name = b"x-panda-plugin";
    let value = b"sample-rust";
    // Safety: host provides `panda_set_header` with pointer+len reads in guest memory.
    unsafe {
        panda_set_header(
            name.as_ptr() as i32,
            name.len() as i32,
            value.as_ptr() as i32,
            value.len() as i32,
        );
    }
    0
}

/// Body hook: replace ASCII `password` with `********` when present.
#[no_mangle]
pub extern "C" fn panda_on_request_body(ptr: i32, len: i32) -> i32 {
    if ptr < 0 || len <= 0 {
        return 0;
    }
    let input = unsafe { std::slice::from_raw_parts(ptr as *const u8, len as usize) };
    let needle = b"password";
    if !input.windows(needle.len()).any(|w| w == needle) {
        return 0;
    }

    let mut out = Vec::with_capacity(input.len());
    let mut i = 0usize;
    while i < input.len() {
        if i + needle.len() <= input.len() && &input[i..i + needle.len()] == needle {
            out.extend_from_slice(b"********");
            i += needle.len();
        } else {
            out.push(input[i]);
            i += 1;
        }
    }
    unsafe {
        panda_set_header(
            b"x-panda-redacted".as_ptr() as i32,
            "x-panda-redacted".len() as i32,
            b"true".as_ptr() as i32,
            4,
        );
        panda_set_body(out.as_ptr() as i32, out.len() as i32);
    }
    0
}
