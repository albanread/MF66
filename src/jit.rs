//! `Mf66Jit` — a thin harness over the JASM AArch64 backend.
//!
//! Wraps `wfasm::native_macos::MacJit` (the `MAP_JIT` loader) driven by the
//! `Loader` builder path: accumulate AArch64 assembly text, bind host externs,
//! then on first lookup assemble it with `wfasm::a64::A64Encoder`, place it,
//! relocate (far calls veneered), flip W^X to executable, and hand back a
//! function pointer. This is the embryo the Forth session (Phase 2+) is built on.

use std::ffi::c_void;

use anyhow::Result;
use wfasm::backend::Loader;
use wfasm::native_macos::MacJit;

/// A JIT unit: AArch64 text + bound externs → callable native code.
pub struct Mf66Jit {
    loader: MacJit,
}

impl Default for Mf66Jit {
    fn default() -> Self {
        Self::new()
    }
}

impl Mf66Jit {
    pub fn new() -> Self {
        Mf66Jit { loader: MacJit::new() }
    }

    /// Append a chunk of AArch64 assembly text (assembled on first lookup).
    pub fn add_asm(&mut self, asm: &str) -> Result<()> {
        self.loader.add_asm(asm)
    }

    /// Bind a host `extern "C"` (AAPCS64) function by name, callable from the
    /// JIT'd code via `bl`/`blr` (the loader veneers it if it lands far away).
    pub fn define_extern(&mut self, name: &str, addr: *const ()) -> Result<()> {
        self.loader.define_extern_fn(name, 0, addr as *mut c_void)
    }

    /// Assemble + place (if needed) and return `name` as a function pointer.
    ///
    /// # Safety
    /// The caller asserts the JIT'd symbol matches `F`'s ABI.
    pub unsafe fn lookup_fn<F: Copy>(&mut self, name: &str) -> Result<F> {
        self.loader.lookup_fn(name)
    }

    /// Resolve `name` to its placed address (assembling + relocating if needed).
    pub fn lookup_addr(&mut self, name: &str) -> Result<u64> {
        self.loader.lookup_addr(name)
    }
}
