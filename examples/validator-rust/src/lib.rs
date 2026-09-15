//! Example sandboxed validator: refuses an `actor` whose `system.hp` is negative. `no_std`,
//! no dependencies (a hand-rolled minimal JSON scan — no `serde-json-core`), no build script.
//! Built directly with `cargo build --target wasm32-unknown-unknown --release`: this crate sits
//! outside the repo's root Cargo workspace, so no workspace-level build script or feature runs
//! against it.
#![no_std]

use core::panic::PanicInfo;

/// A bump allocator over a fixed static arena — the whole guest ABI's `alloc` needs no
/// deallocation (one call per validation, one validation per fresh `Store`).
const ARENA_SIZE: usize = 64 * 1024;
static mut ARENA: [u8; ARENA_SIZE] = [0; ARENA_SIZE];
static mut NEXT: usize = 0;

// #region alloc
/// Reserves `len` bytes from the static arena and returns their LINEAR-MEMORY ADDRESS (never a
/// bare offset — the guest's stack and other statics share the same address space, so only a
/// pointer derived from `ARENA` itself is guaranteed not to collide), or `-1` if the arena is
/// exhausted (the host treats any negative return as a bad pointer fault).
#[no_mangle]
pub extern "C" fn alloc(len: i32) -> i32 {
    // SAFETY: single-threaded WASM guest, one call per Store per validation — no concurrent
    // access to `NEXT`/`ARENA` is possible.
    unsafe {
        let len = len as usize;
        if NEXT + len > ARENA_SIZE {
            return -1;
        }
        // `addr_of_mut!` takes the arena's address without creating a reference to a mutable
        // static (the `static_mut_refs` deny lint); the host writes through this raw address.
        let base = core::ptr::addr_of_mut!(ARENA) as *mut u8;
        let ptr = base.add(NEXT) as i32;
        NEXT += len;
        ptr
    }
}

// #endregion alloc
// #region validate
/// Reads the `ValidatorInput` JSON at `(ptr, len)`, hand-scans for `"hp":<number>` inside the
/// top-level `system` object, and refuses when that number is negative. Any other document
/// (no `hp` key, or `hp >= 0`) is accepted. This is intentionally a minimal, forgiving scan —
/// not a general JSON parser — matching the guide's stated scope; the one discipline it keeps
/// absolutely is ANCHORING to the `system` value's span (`find_system_span`), so a `"hp":`
/// substring anywhere else in the input (a document name, another band) is never mistaken
/// for the field it judges.
#[no_mangle]
pub extern "C" fn validate(ptr: i32, len: i32) -> i32 {
    let bytes = unsafe { core::slice::from_raw_parts(ptr as *const u8, len as usize) };
    match find_hp(bytes) {
        Some(hp) if hp < 0 => 1,
        _ => 0,
    }
}

/// Locates the `"system":` key's VALUE and returns its `(start, end)` byte span — the
/// balanced-brace extent of the object that follows, string-aware so a `}` inside a string
/// value doesn't end the span early. `None` when the key or a well-formed object value is
/// absent.
fn find_system_span(bytes: &[u8]) -> Option<(usize, usize)> {
    const KEY: &[u8] = b"\"system\":";
    let mut i = bytes.windows(KEY.len()).position(|w| w == KEY)? + KEY.len();
    while bytes.get(i).is_some_and(u8::is_ascii_whitespace) {
        i += 1;
    }
    if bytes.get(i) != Some(&b'{') {
        return None;
    }
    let start = i;
    let mut depth: u32 = 0;
    let mut in_string = false;
    let mut escaped = false;
    let mut j = i;
    while let Some(&b) = bytes.get(j) {
        j += 1;
        if in_string {
            if escaped {
                escaped = false;
            } else if b == b'\\' {
                escaped = true;
            } else if b == b'"' {
                in_string = false;
            }
            continue;
        }
        match b {
            b'"' => in_string = true,
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some((start, j));
                }
            }
            _ => {}
        }
    }
    None
}

/// Byte-scans for the literal substring `"hp":` WITHIN the `system` value's span and parses
/// the signed integer that follows (optional leading `-`, then ASCII digits, stopping at the
/// first non-digit). Returns `None` if the key is absent or the following text is not a
/// recognizable integer.
fn find_hp(bytes: &[u8]) -> Option<i64> {
    const NEEDLE: &[u8] = b"\"hp\":";
    let (start, end) = find_system_span(bytes)?;
    let span = &bytes[start..end];
    let pos = span.windows(NEEDLE.len()).position(|w| w == NEEDLE)? + NEEDLE.len() + start;
    let mut i = pos;
    let negative = bytes.get(i) == Some(&b'-');
    if negative {
        i += 1;
    }
    let digits_start = i;
    while bytes.get(i).is_some_and(u8::is_ascii_digit) {
        i += 1;
    }
    if i == digits_start {
        return None;
    }
    let mut value: i64 = 0;
    for &b in &bytes[digits_start..i] {
        value = value * 10 + i64::from(b - b'0');
    }
    Some(if negative { -value } else { value })
}

// #endregion validate
// #region reason
/// Static reason text for the one refusal case this validator authors.
static REASON: &[u8] = b"system.hp must not be negative";

/// The refusal reason's address, for the host's `reason_ptr`/`reason_len` read after a
/// non-zero `validate` return.
#[no_mangle]
pub extern "C" fn reason_ptr() -> i32 {
    REASON.as_ptr() as i32
}

/// The refusal reason's byte length.
#[no_mangle]
pub extern "C" fn reason_len() -> i32 {
    REASON.len() as i32
}

// #endregion reason

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    loop {}
}
