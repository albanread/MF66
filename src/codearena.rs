//! `CodeArena` — a `MAP_JIT` executable region for compiled colon-word bodies.
//!
//! Bodies are accumulated as instruction words in Rust, then `commit`ted in one
//! W^X cycle: flip the thread to write, copy the words in, flip back to execute,
//! invalidate the icache for the new range. `pthread_jit_write_protect_np` is
//! per-thread and covers all `MAP_JIT` pages (incl. `MacJit`'s), so commits must
//! not interleave with running JIT code — which holds, since a definition is
//! compiled (no execution) and only then run.

use std::ffi::c_void;
use std::ptr;

use anyhow::{bail, Result};

extern "C" {
    fn mmap(addr: *mut c_void, len: usize, prot: i32, flags: i32, fd: i32, offset: i64) -> *mut c_void;
    fn munmap(addr: *mut c_void, len: usize) -> i32;
    fn pthread_jit_write_protect_np(enabled: i32);
    fn sys_icache_invalidate(start: *mut c_void, len: usize);
}

const PROT_READ: i32 = 1;
const PROT_WRITE: i32 = 2;
const PROT_EXEC: i32 = 4;
const MAP_PRIVATE: i32 = 0x0002;
const MAP_ANON: i32 = 0x1000;
const MAP_JIT: i32 = 0x0800;

pub struct CodeArena {
    region: *mut u8,
    cap: usize,
    cursor: usize, // bytes used
}

impl CodeArena {
    pub fn with_capacity(cap: usize) -> Result<Self> {
        let cap = (cap + 0xFFFF) & !0xFFFF; // round up to 64 KB
        let region = unsafe {
            mmap(
                ptr::null_mut(),
                cap,
                PROT_READ | PROT_WRITE | PROT_EXEC,
                MAP_PRIVATE | MAP_ANON | MAP_JIT,
                -1,
                0,
            )
        };
        if region.is_null() || region as isize == -1 {
            bail!("mmap(MAP_JIT) failed for the code arena");
        }
        Ok(CodeArena { region: region as *mut u8, cap, cursor: 0 })
    }

    /// Copy `words` in as executable code (16-byte aligned); return its address.
    pub fn commit(&mut self, words: &[u32]) -> Result<u64> {
        let start = (self.cursor + 15) & !15;
        let bytes = words.len() * 4;
        if start + bytes > self.cap {
            bail!("code arena exhausted ({} + {bytes} > {})", start, self.cap);
        }
        let addr = unsafe { self.region.add(start) };
        unsafe {
            pthread_jit_write_protect_np(0); // writable
            for (i, w) in words.iter().enumerate() {
                (addr as *mut u32).add(i).write(*w);
            }
            pthread_jit_write_protect_np(1); // executable
            sys_icache_invalidate(addr as *mut c_void, bytes);
        }
        self.cursor = start + bytes;
        Ok(addr as u64)
    }
}

impl Drop for CodeArena {
    fn drop(&mut self) {
        unsafe { munmap(self.region as *mut c_void, self.cap) };
    }
}
