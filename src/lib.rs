//! # MF66 — Apple Silicon (macOS arm64) token-IR optimizing STC Forth
//!
//! A re-implementation of **WF66** (the Windows x86-64 compiler) for Apple
//! Silicon, JIT-compiled through the **LLVM-free JASM AArch64 backend**
//! (`wfasm::a64::A64Encoder` + `wfasm::native_macos::MacJit`). It reuses WF66's
//! architecture-neutral front-end + token-IR reducer and MacNCL's GC + REPL/IDE;
//! the new work is the AArch64 lowering/back-end and the STC kernel.
//!
//! See `docs/design/mf66-apple-silicon.md` for the full plan and the verified
//! ABI. This crate is at **Phase 0** (substrate smoke test).

#![cfg(target_os = "macos")]

pub mod abi;
pub mod jit;

pub use jit::Mf66Jit;
