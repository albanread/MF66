//! A signal-safe crash handler for the JIT.
//!
//! JIT'd Forth can fault (a stack underflow that dereferences a small integer as
//! a pointer, a bad `@`/`!` address, …) — a SIGSEGV/SIGBUS that would otherwise
//! abort the process with no context. [`guard`] runs a closure with a handler
//! armed: a fault is captured ([`CrashInfo`]: signal + faulting address) and
//! turned into an `Err` via `siglongjmp`, so a harness can report the offending
//! input and carry on instead of dying. Outside `guard`, the default disposition
//! is restored and the fault re-raised, so ordinary crashes still abort normally.
//!
//! Single-threaded by design (MF66 runs the JIT on one thread; see `.cargo`).
//! Recovery abandons the faulting stack frame without running Rust destructors —
//! acceptable for a debugging / test-harness tool, not for production control flow.

use std::os::raw::{c_int, c_void};
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicUsize, Ordering};
use std::sync::Once;

/// Details captured at the point of a fault.
#[derive(Clone, Copy, Debug, Default)]
pub struct CrashInfo {
    pub signal: c_int,
    pub fault_addr: usize,
}

impl std::fmt::Display for CrashInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let name = match self.signal {
            libc::SIGSEGV => "SIGSEGV",
            libc::SIGBUS => "SIGBUS",
            n => return write!(f, "signal {n} at {:#x}", self.fault_addr),
        };
        write!(f, "{name} (fault address {:#x})", self.fault_addr)
    }
}

// Recovery state. Single-threaded, so process-global statics suffice.
const JB_WORDS: usize = 96; // > darwin arm64 sigjmp_buf (~49 ints)
static mut JMP: [u64; JB_WORDS] = [0; JB_WORDS];
static ARMED: AtomicBool = AtomicBool::new(false);
static SIG: AtomicI32 = AtomicI32::new(0);
static FAULT: AtomicUsize = AtomicUsize::new(0);
static INSTALL: Once = Once::new();

extern "C" {
    fn sigsetjmp(env: *mut c_void, savemask: c_int) -> c_int;
    fn siglongjmp(env: *mut c_void, val: c_int) -> !;
}

extern "C" fn handler(sig: c_int, info: *mut libc::siginfo_t, _uctx: *mut c_void) {
    let addr = unsafe {
        if info.is_null() {
            0
        } else {
            (*info).si_addr() as usize
        }
    };
    SIG.store(sig, Ordering::SeqCst);
    FAULT.store(addr, Ordering::SeqCst);
    if ARMED.swap(false, Ordering::SeqCst) {
        // Recover: jump back into `guard`, which returns Err.
        unsafe { siglongjmp(core::ptr::addr_of_mut!(JMP) as *mut c_void, 1) }
    }
    // Not armed → restore the default handler and let the instruction re-fault,
    // preserving normal abort-with-core behavior outside a guard.
    unsafe {
        let mut sa: libc::sigaction = std::mem::zeroed();
        sa.sa_sigaction = libc::SIG_DFL;
        libc::sigaction(sig, &sa, core::ptr::null_mut());
    }
}

fn install() {
    INSTALL.call_once(|| unsafe {
        let mut sa: libc::sigaction = std::mem::zeroed();
        sa.sa_sigaction = handler as usize;
        sa.sa_flags = libc::SA_SIGINFO;
        libc::sigemptyset(&mut sa.sa_mask);
        libc::sigaction(libc::SIGSEGV, &sa, core::ptr::null_mut());
        libc::sigaction(libc::SIGBUS, &sa, core::ptr::null_mut());
    });
}

/// Run `f` with the crash handler armed. Returns `Ok(f())` normally, or
/// `Err(CrashInfo)` if `f` faulted (SIGSEGV/SIGBUS) — recovered via `siglongjmp`.
pub fn guard<R>(f: impl FnOnce() -> R) -> Result<R, CrashInfo> {
    install();
    unsafe {
        if sigsetjmp(core::ptr::addr_of_mut!(JMP) as *mut c_void, 1) == 0 {
            ARMED.store(true, Ordering::SeqCst);
            let r = f();
            ARMED.store(false, Ordering::SeqCst);
            Ok(r)
        } else {
            // Returned here via siglongjmp from the handler — a fault occurred.
            Err(CrashInfo {
                signal: SIG.load(Ordering::SeqCst),
                fault_addr: FAULT.load(Ordering::SeqCst),
            })
        }
    }
}
