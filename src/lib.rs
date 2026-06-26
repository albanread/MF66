//! # MF66 — Apple Silicon (macOS arm64) token-IR optimizing STC Forth
//!
//! A re-implementation of **WF66** (the Windows x86-64 compiler) for Apple
//! Silicon, JIT-compiled through the **LLVM-free JASM AArch64 backend**
//! (`wfasm::a64::A64Encoder` + `wfasm::native_macos::MacJit`). It reuses WF66's
//! architecture-neutral front-end + token-IR reducer and MacNCL's GC + REPL/IDE;
//! the new work is the AArch64 lowering/back-end and the STC kernel.
//!
//! See `docs/design/mf66-apple-silicon.md` for the full plan and the verified
//! ABI. This crate is at **Phase 1** (kernel macro library + headless boot path).

#![cfg(target_os = "macos")]

pub mod abi;
pub mod aenc;
pub mod asm_macros;
pub mod codearena;
pub mod jit;
pub mod primitives;
pub mod runtime;
pub mod session;

pub use jit::Mf66Jit;
pub use session::Mf66Session;
