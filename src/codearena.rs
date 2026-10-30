//! `CodeArena` — a `MAP_JIT` executable region for compiled colon-word bodies.
//!
//! Bodies are accumulated as instruction words in Rust, then `commit`ted in one
//! W^X cycle: flip the thread to write, copy the words in, flip back to execute,
//! invalidate the icache for the new range. `pthread_jit_write_protect_np` is
//! per-thread and covers all `MAP_JIT` pages (incl. `MacJit`'s), so commits must
//! not interleave with running JIT code — which holds, since a definition is
//! compiled (no execution) and only then run.


use anyhow::{bail, Result};

use wfasm::jitmem;

pub struct CodeArena {
    region: *mut u8,
    cap: usize,
    cursor: usize, // bytes used
}

impl CodeArena {
    pub fn with_capacity(cap: usize) -> Result<Self> {
        let cap = (cap + 0xFFFF) & !0xFFFF; // round up to 64 KB
        let region = jitmem::map_rwx(cap);
        if region.is_null() {
            bail!("mmap(MAP_JIT) failed for the code arena");
        }
        Ok(CodeArena { region, cap, cursor: 0 })
    }

    /// The address the next `commit` will return (16-byte aligned) — used by
    /// `recurse` to call a definition before its body is committed.
    pub fn next_addr(&self) -> u64 {
        let start = (self.cursor + 15) & !15;
        unsafe { self.region.add(start) as u64 }
    }

    /// Is `addr` a body committed to this arena? Distinguishes a colon/CODE word
    /// (which nests/unnests cleanly) from a kernel primitive — used to gate the
    /// tail-call optimization, which is only sound for the former.
    pub fn contains(&self, addr: u64) -> bool {
        let base = self.region as u64;
        addr >= base && addr < base + self.cap as u64
    }

    /// Current write cursor — a checkpoint for `marker`/`forget` rollback.
    pub fn checkpoint(&self) -> usize {
        self.cursor
    }

    /// Roll the write cursor back to a prior `checkpoint` (reclaims code space).
    pub fn reset_to(&mut self, cursor: usize) {
        self.cursor = cursor;
    }

    /// Copy `words` in as executable code (16-byte aligned); return its address.
    pub fn commit(&mut self, words: &[u32]) -> Result<u64> {
        let start = (self.cursor + 15) & !15;
        let bytes = words.len() * 4;
        if start + bytes > self.cap {
            bail!("code arena exhausted ({} + {bytes} > {})", start, self.cap);
        }
        let addr = unsafe { self.region.add(start) };
        jitmem::write_protect(false); // writable
        for (i, w) in words.iter().enumerate() {
            unsafe { (addr as *mut u32).add(i).write(*w) };
        }
        jitmem::write_protect(true); // executable
        jitmem::icache_invalidate(addr, bytes);
        self.cursor = start + bytes;
        Ok(addr as u64)
    }
}

impl Drop for CodeArena {
    fn drop(&mut self) {
        jitmem::unmap(self.region, self.cap);
    }
}
