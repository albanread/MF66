//! `Mf66Session` — Phase 1 boot harness.
//!
//! Allocates the Forth region (data stack / return stack / user area / locals),
//! assembles the AArch64 kernel through the front-end + `MacJit`, seeds the user
//! area, and drives primitives through `forth_main` using the memory wire-format
//! (`push`/`call`/`stack`), mirroring WF66's `Wf64Session`.

use std::alloc::{alloc_zeroed, dealloc, Layout};
use std::path::PathBuf;

use anyhow::{Context, Result};
use wfasm::backend::Loader;
use wfasm::native_macos::MacJit;
use wfasm::Assembler;

// ── Region layout (byte offsets within the allocation) ───────────────────
const REGION_SIZE: usize = 4 * 1024 * 1024; // 4 MB
const DSTACK_TOP: usize = 0x0008_0000; // data stack: 0..0x80000, grows down from here
const RSTACK_TOP: usize = 0x0010_0000; // return stack: 0x80000..0x100000, grows down
const USER_BASE: usize = 0x0010_0000; // user area: 0x100000..0x180000
const LOCALS_TOP: usize = 0x0020_0000; // locals: 0x180000..0x200000, grows down

// ── User-area offsets (must match kernel/macros.masm) ────────────────────
const USER_HOST_RSP: usize = 0x00;
const USER_DSP_SAVE: usize = 0x08;
const USER_SP0: usize = 0x10;
const USER_RSP_CURRENT: usize = 0x18;
const USER_LP0: usize = 0x20;
const USER_FTOS_SAVE: usize = 0x28;
/// Scratch region inside the user area for memory/string-primitive tests
/// (`push_pad`/`poke`/`expect_bytes`). Sits above the user-variable table with
/// headroom for the full Phase 2+ table below it.
const USER_PAD: u64 = 0x800;

const CELL: usize = 8;

/// `forth_main(target_xt, logical_dsp_in, rsp_top, user_base) -> 0`.
type ForthMain = extern "C" fn(u64, u64, u64, u64) -> u64;

pub struct Mf66Session {
    // Keep the loader alive: the JIT'd code lives in its arena.
    _jit: MacJit,
    forth_main: ForthMain,

    region: *mut u8,
    layout: Layout,

    dstack_top: u64,
    rstack_top: u64,
    user_base: u64,
    /// Current logical data-stack pointer (== dstack_top when empty).
    current_dsp: u64,
}

impl Mf66Session {
    /// Boot with only the built-in runtime externs.
    pub fn new() -> Result<Self> {
        Self::with_externs(&[])
    }

    /// Boot, binding `extra` host externs (name → `extern "C"` address) in
    /// addition to the built-ins. Externs must be bound before the kernel is
    /// assembled, so they are supplied up front.
    pub fn with_externs(extra: &[(&str, *const ())]) -> Result<Self> {
        // 1. Expand the kernel macros to AArch64 text.
        let mut asm = Assembler::new();
        asm.register_macro("stk", crate::asm_macros::stk);
        let main = kernel_path();
        let text = asm
            .assemble_file(&main)
            .map_err(|e| anyhow::anyhow!("assemble {}: {e}", main.display()))?;

        // 2. Load into MAP_JIT memory, binding host externs first.
        let mut jit = MacJit::new();
        for (name, addr) in crate::runtime::externs().iter().chain(extra.iter()) {
            jit.define_extern_fn(name, 0, *addr as *mut std::ffi::c_void)
                .with_context(|| format!("bind extern {name}"))?;
        }
        jit.add_asm(&text)?;
        let forth_main: ForthMain = unsafe { jit.lookup_fn("forth_main")? };

        // 3. Allocate + seed the Forth region.
        let layout = Layout::from_size_align(REGION_SIZE, 4096).unwrap();
        let region = unsafe { alloc_zeroed(layout) };
        if region.is_null() {
            anyhow::bail!("region allocation failed");
        }
        let base = region as u64;
        let dstack_top = base + DSTACK_TOP as u64;
        let rstack_top = base + RSTACK_TOP as u64;
        let user_base = base + USER_BASE as u64;

        let mut s = Mf66Session {
            _jit: jit,
            forth_main,
            region,
            layout,
            dstack_top,
            rstack_top,
            user_base,
            current_dsp: dstack_top,
        };

        s.write_user(USER_RSP_CURRENT, rstack_top);
        s.write_user(USER_LP0, base + LOCALS_TOP as u64);
        s.write_user(USER_SP0, dstack_top);
        s.write_user(USER_FTOS_SAVE, 0);
        s.write_user(USER_HOST_RSP, 0);
        s.write_user(USER_DSP_SAVE, dstack_top);
        Ok(s)
    }

    // ── Stack API (mirrors Wf64Session) ─────────────────────────────────
    /// Push a cell.
    pub fn push(&mut self, v: i64) {
        self.current_dsp -= CELL as u64;
        unsafe { (self.current_dsp as *mut u64).write(v as u64) };
    }

    /// Data stack contents, top-first.
    pub fn stack(&self) -> Vec<i64> {
        let depth = self.depth();
        (0..depth)
            .map(|i| unsafe { (self.current_dsp as *const u64).add(i).read() as i64 })
            .collect()
    }

    pub fn depth(&self) -> usize {
        ((self.dstack_top - self.current_dsp) / CELL as u64) as usize
    }

    /// Resolve a primitive's asm symbol to its execution token (code address).
    /// Used by the corpus harness for NYIMP detection.
    pub fn xt_of(&mut self, asm_sym: &str) -> Result<u64> {
        self._jit
            .lookup_addr(asm_sym)
            .with_context(|| format!("xt_of({asm_sym})"))
    }

    /// Base of the user-area scratch (PAD) region.
    pub fn pad_base(&self) -> u64 {
        self.user_base + USER_PAD
    }

    /// Invoke a primitive by its asm symbol through `forth_main`.
    pub fn call(&mut self, asm_sym: &str) -> Result<()> {
        let xt = self.xt_of(asm_sym)?;
        (self.forth_main)(xt, self.current_dsp, self.rstack_top, self.user_base);
        self.current_dsp = self.read_user(USER_DSP_SAVE);
        Ok(())
    }

    /// Clear the data stack (Phase 1 reset).
    pub fn reset(&mut self) {
        self.current_dsp = self.dstack_top;
        self.write_user(USER_DSP_SAVE, self.dstack_top);
    }

    // ── helpers ──────────────────────────────────────────────────────────
    fn read_user(&self, off: usize) -> u64 {
        unsafe { ((self.user_base + off as u64) as *const u64).read() }
    }
    fn write_user(&mut self, off: usize, v: u64) {
        unsafe { ((self.user_base + off as u64) as *mut u64).write(v) };
    }
}

impl Drop for Mf66Session {
    fn drop(&mut self) {
        unsafe { dealloc(self.region, self.layout) };
    }
}

/// Locate `kernel/main.masm` relative to the crate root.
fn kernel_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("kernel/main.masm")
}
