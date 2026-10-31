//! Host hooks: how an embedding host (the BOX's native module, box/) gives the
//! Forth its output, input, memory, files and system calls. On the Mac nothing
//! is set and every hook falls back to what MF66 always did (the capture
//! buffer, `alloc_zeroed`, libc `malloc`, Rust's `std::fs`).
//!
//! The table is C-shaped (`#[repr(C)]`) so a C host can fill it in; every
//! entry is optional. See box/mf66_box.h for the C side.

use std::sync::atomic::{AtomicPtr, Ordering};

pub mod console;

/// What a host may supply. Strings are (pointer, length); nothing is kept.
#[repr(C)]
pub struct Hooks {
    /// Print bytes as they are produced (instead of the capture buffer).
    pub write: Option<extern "C" fn(*const u8, usize)>,
    /// `key`: one character, or -1 (Escape / end of input).
    pub key: Option<extern "C" fn() -> i64>,
    /// `key?`: non-zero if a key is waiting.
    pub key_ready: Option<extern "C" fn() -> i64>,
    /// `accept`: read a line into (buf, max): its length, or -1.
    pub read_line: Option<extern "C" fn(*mut u8, usize) -> i64>,
    /// The session's data region (zeroed), and giving it back.
    pub region_alloc: Option<extern "C" fn(usize) -> *mut u8>,
    pub region_free: Option<extern "C" fn(*mut u8, usize)>,
    /// `allocate` / `free` / `resize` (null = failure).
    pub mem_alloc: Option<extern "C" fn(usize) -> *mut u8>,
    pub mem_free: Option<extern "C" fn(*mut u8)>,
    pub mem_resize: Option<extern "C" fn(*mut u8, usize) -> *mut u8>,
    /// A system call: number, ten registers in and out, the flags out (N Z C V
    /// as bits 3..0). The result is 0, or the address of an error block.
    pub swi: Option<extern "C" fn(u64, *mut u64, *mut u64) -> u64>,
    /// A system call's number from its name, or -1.
    pub swi_number: Option<extern "C" fn(*const u8, usize) -> i64>,
    /// `ms`: wait this many milliseconds.
    pub ms: Option<extern "C" fn(u64)>,
    /// Files by the host's own names. `file_open` modes: 1 read, 2 write
    /// (existing), 3 update, 4 create (truncate); the result is a handle, 0 if
    /// it failed. Reads and writes return the bytes moved, or -1.
    pub file_open: Option<extern "C" fn(*const u8, usize, u64) -> u64>,
    pub file_close: Option<extern "C" fn(u64) -> i64>,
    pub file_read: Option<extern "C" fn(u64, *mut u8, usize) -> i64>,
    pub file_write: Option<extern "C" fn(u64, *const u8, usize) -> i64>,
    /// Position and extent: get with `set` false, or set; -1 on failure.
    pub file_ptr: Option<extern "C" fn(u64, u64, i64) -> i64>,
    pub file_extent: Option<extern "C" fn(u64) -> i64>,
    pub file_delete: Option<extern "C" fn(*const u8, usize) -> i64>,
    pub file_rename: Option<extern "C" fn(*const u8, usize, *const u8, usize) -> i64>,
}

static HOOKS: AtomicPtr<Hooks> = AtomicPtr::new(std::ptr::null_mut());

/// Install the host's hooks (null removes them). The table must outlive every
/// session.
pub fn set_hooks(h: *const Hooks) {
    HOOKS.store(h as *mut Hooks, Ordering::SeqCst);
}

/// The installed hooks, if any.
pub fn hooks() -> Option<&'static Hooks> {
    let p = HOOKS.load(Ordering::SeqCst);
    if p.is_null() {
        None
    } else {
        Some(unsafe { &*p })
    }
}

/// The host's data region, if it gives one.
pub fn region_alloc(size: usize) -> Option<*mut u8> {
    let p = (hooks()?.region_alloc?)(size);
    (!p.is_null()).then_some(p)
}

/// Give a region [`region_alloc`] made back to the host.
pub fn region_free(p: *mut u8, size: usize) {
    if let Some(f) = hooks().and_then(|h| h.region_free) {
        f(p, size);
    }
}
