//! MF66 host runtime functions, bound into the kernel as AAPCS64 externs.
//!
//! Output goes to a thread-local capture buffer (the session runs single-threaded
//! for JIT, so one thread-local suffices); `Mf66Session::eval_out` clears it,
//! runs, and takes the result. This mirrors WF66's `rt_emit`/`rt_type`/
//! `rt_print_int` and their exact formatting so the eval corpus matches.

use std::cell::RefCell;

thread_local! {
    static CAPTURE: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
}

/// Clear the capture buffer (before an `eval`).
pub fn capture_clear() {
    CAPTURE.with(|b| b.borrow_mut().clear());
}

/// Append a host string to the capture buffer (e.g. the REPL's ` ok`).
pub fn capture_str(s: &str) {
    CAPTURE.with(|b| b.borrow_mut().extend_from_slice(s.as_bytes()));
}

/// Take the captured output as a string (after an `eval`).
pub fn capture_take() -> String {
    CAPTURE.with(|b| {
        let bytes = std::mem::take(&mut *b.borrow_mut());
        String::from_utf8_lossy(&bytes).into_owned()
    })
}

/// `emit ( c -- )` host side: append one byte.
pub extern "C" fn rt_emit(ch: u64) -> u64 {
    CAPTURE.with(|b| b.borrow_mut().push(ch as u8));
    0
}

/// `type ( addr u -- )` host side: append `len` bytes from `addr`.
pub extern "C" fn rt_type(addr: u64, len: u64) -> u64 {
    if addr != 0 && len > 0 {
        let slice = unsafe { std::slice::from_raw_parts(addr as *const u8, len as usize) };
        CAPTURE.with(|b| b.borrow_mut().extend_from_slice(slice));
    }
    0
}

/// `. ( n -- )` host side: signed number in `base`, then ONE trailing space
/// (byte-identical to WF66's `rt_print_int`).
pub extern "C" fn rt_print_int(n: u64, base: u64) -> u64 {
    let s = n as i64;
    let b = if (2..=36).contains(&base) { base as u32 } else { 10 };
    let out = if b == 10 {
        format!("{s} ")
    } else {
        let (neg, mag) = if s < 0 {
            (true, (s as i128).unsigned_abs())
        } else {
            (false, s as u128)
        };
        let mut digits = Vec::with_capacity(24);
        let mut v = mag;
        if v == 0 {
            digits.push(b'0');
        }
        while v > 0 {
            let d = (v % b as u128) as u8;
            digits.push(if d < 10 { b'0' + d } else { b'A' + (d - 10) });
            v /= b as u128;
        }
        let mut out = String::new();
        if neg {
            out.push('-');
        }
        for &d in digits.iter().rev() {
            out.push(d as char);
        }
        out.push(' ');
        out
    };
    CAPTURE.with(|b| b.borrow_mut().extend_from_slice(out.as_bytes()));
    0
}

/// `rt_double(n) -> 2n` — a Phase-1 host-call smoke target for `aapcs_call`.
pub extern "C" fn rt_double(n: u64) -> u64 {
    n.wrapping_mul(2)
}

/// `f.` — print a float in Forth style (shortest round-trip, trailing space).
pub extern "C" fn rt_print_float(x: f64) {
    capture_str(&format!("{x} "));
}

// Transcendental math for the FP word set (libm via Rust's f64 methods, which
// lower to the platform math library). One-argument:
pub extern "C" fn rt_fsin(x: f64) -> f64 { x.sin() }
pub extern "C" fn rt_fcos(x: f64) -> f64 { x.cos() }
pub extern "C" fn rt_ftan(x: f64) -> f64 { x.tan() }
pub extern "C" fn rt_fexp(x: f64) -> f64 { x.exp() }
pub extern "C" fn rt_fln(x: f64) -> f64 { x.ln() }
pub extern "C" fn rt_flog(x: f64) -> f64 { x.log10() }
pub extern "C" fn rt_fatan(x: f64) -> f64 { x.atan() }
pub extern "C" fn rt_fasin(x: f64) -> f64 { x.asin() }
pub extern "C" fn rt_facos(x: f64) -> f64 { x.acos() }
// Two-argument (a in d0, b in d1):
pub extern "C" fn rt_fpow(a: f64, b: f64) -> f64 { a.powf(b) }
pub extern "C" fn rt_fatan2(a: f64, b: f64) -> f64 { a.atan2(b) }

// ── File-Access wordset host side ────────────────────────────────────────
// A Forth fid is (index+1) into a thread-local table; 0 is never valid. Each
// rt_* returns the ANS `ior` (0 = success). Words with a second numeric output
// (read-file u2, file-position/size, read-line u2+flag) take an out-pointer the
// kernel supplies (a scratch cell in the user PAD) and write that value through it.
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};

thread_local! {
    static FILES: RefCell<Vec<Option<File>>> = const { RefCell::new(Vec::new()) };
}

fn path_from(addr: u64, len: u64) -> String {
    if addr == 0 || len == 0 {
        return String::new();
    }
    let slice = unsafe { std::slice::from_raw_parts(addr as *const u8, len as usize) };
    String::from_utf8_lossy(slice).into_owned()
}

fn install(f: File) -> u64 {
    FILES.with(|t| {
        let mut t = t.borrow_mut();
        if let Some(i) = t.iter().position(|s| s.is_none()) {
            t[i] = Some(f);
            (i + 1) as u64
        } else {
            t.push(Some(f));
            t.len() as u64
        }
    })
}

fn with_file<R>(fid: u64, f: impl FnOnce(&mut File) -> R) -> Option<R> {
    if fid == 0 {
        return None;
    }
    FILES.with(|t| {
        let mut t = t.borrow_mut();
        t.get_mut((fid - 1) as usize).and_then(|slot| slot.as_mut()).map(f)
    })
}

fn fam_opts(fam: u64) -> Option<OpenOptions> {
    let mut o = OpenOptions::new();
    match fam & 7 {
        1 => { o.read(true); }
        2 => { o.write(true); }
        3 => { o.read(true).write(true); }
        _ => return None,
    }
    Some(o)
}

/// open-file ( c-addr u fam -- fid ior ); kernel passes &fid_out (PAD).
pub extern "C" fn rt_open_file(addr: u64, len: u64, fam: u64, fid_out: *mut u64) -> u64 {
    let opts = match fam_opts(fam) {
        Some(o) => o,
        None => { unsafe { *fid_out = 0 }; return (-36i64) as u64; }
    };
    match opts.open(path_from(addr, len)) {
        Ok(f) => { let fid = install(f); unsafe { *fid_out = fid }; 0 }
        Err(_) => { unsafe { *fid_out = 0 }; (-62i64) as u64 }
    }
}

/// create-file ( c-addr u fam -- fid ior ); truncate-or-create, read+write.
pub extern "C" fn rt_create_file(addr: u64, len: u64, fam: u64, fid_out: *mut u64) -> u64 {
    if fam_opts(fam).is_none() {
        unsafe { *fid_out = 0 };
        return (-36i64) as u64;
    }
    let mut o = OpenOptions::new();
    o.read(true).write(true).create(true).truncate(true);
    match o.open(path_from(addr, len)) {
        Ok(f) => { let fid = install(f); unsafe { *fid_out = fid }; 0 }
        Err(_) => { unsafe { *fid_out = 0 }; (-62i64) as u64 }
    }
}

/// close-file ( fid -- ior )
pub extern "C" fn rt_close_file(fid: u64) -> u64 {
    if fid == 0 {
        return (-63i64) as u64;
    }
    FILES.with(|t| {
        let mut t = t.borrow_mut();
        match t.get_mut((fid - 1) as usize) {
            Some(slot) if slot.is_some() => { *slot = None; 0 }
            _ => (-63i64) as u64,
        }
    })
}

/// read-file ( c-addr u1 fid -- u2 ior ); kernel passes &u2_out.
pub extern "C" fn rt_read_file(addr: u64, u1: u64, fid: u64, u2_out: *mut u64) -> u64 {
    unsafe { *u2_out = 0 };
    if addr == 0 {
        return (-64i64) as u64;
    }
    let buf = unsafe { std::slice::from_raw_parts_mut(addr as *mut u8, u1 as usize) };
    match with_file(fid, |f| f.read(buf)) {
        Some(Ok(n)) => { unsafe { *u2_out = n as u64 }; 0 }
        _ => (-64i64) as u64,
    }
}

/// write-file ( c-addr u fid -- ior )
pub extern "C" fn rt_write_file(addr: u64, u: u64, fid: u64) -> u64 {
    if addr == 0 {
        return (-65i64) as u64;
    }
    let buf = unsafe { std::slice::from_raw_parts(addr as *const u8, u as usize) };
    match with_file(fid, |f| f.write_all(buf)) {
        Some(Ok(())) => 0,
        _ => (-65i64) as u64,
    }
}

/// file-position ( fid -- ud ior ); kernel passes &pos_out.
pub extern "C" fn rt_file_position(fid: u64, pos_out: *mut u64) -> u64 {
    unsafe { *pos_out = 0 };
    match with_file(fid, |f| f.stream_position()) {
        Some(Ok(p)) => { unsafe { *pos_out = p }; 0 }
        _ => (-67i64) as u64,
    }
}

/// file-size ( fid -- ud ior ); kernel passes &size_out. Does not move the pointer.
pub extern "C" fn rt_file_size(fid: u64, size_out: *mut u64) -> u64 {
    unsafe { *size_out = 0 };
    match with_file(fid, |f| f.metadata().map(|m| m.len())) {
        Some(Ok(sz)) => { unsafe { *size_out = sz }; 0 }
        _ => (-68i64) as u64,
    }
}

/// reposition-file ( ud fid -- ior )  (kernel passes pos = low cell of ud)
pub extern "C" fn rt_reposition_file(pos: u64, fid: u64) -> u64 {
    match with_file(fid, |f| f.seek(SeekFrom::Start(pos))) {
        Some(Ok(_)) => 0,
        _ => (-69i64) as u64,
    }
}

/// write-line ( c-addr u fid -- ior ); writes the buffer then CR/LF.
pub extern "C" fn rt_write_line(addr: u64, u: u64, fid: u64) -> u64 {
    let body: &[u8] = if addr != 0 && u > 0 {
        unsafe { std::slice::from_raw_parts(addr as *const u8, u as usize) }
    } else {
        &[]
    };
    match with_file(fid, |f| f.write_all(body).and_then(|_| f.write_all(b"\r\n"))) {
        Some(Ok(())) => 0,
        _ => (-65i64) as u64,
    }
}

/// read-line ( c-addr u1 fid -- u2 flag ior ); kernel passes &u2_out, &flag_out.
pub extern "C" fn rt_read_line(
    addr: u64, u1: u64, fid: u64, u2_out: *mut u64, flag_out: *mut u64,
) -> u64 {
    unsafe { *u2_out = 0; *flag_out = 0; }
    if addr == 0 {
        return (-64i64) as u64;
    }
    let buf = unsafe { std::slice::from_raw_parts_mut(addr as *mut u8, u1 as usize) };
    let mut pos: usize = 0;
    let mut any = false;
    loop {
        if pos >= u1 as usize {
            unsafe { *u2_out = pos as u64; *flag_out = u64::MAX };
            return 0;
        }
        let mut one = [0u8; 1];
        match with_file(fid, |f| f.read(&mut one)) {
            Some(Ok(0)) => {
                if pos > 0 || any {
                    unsafe { *u2_out = pos as u64; *flag_out = u64::MAX };
                } else {
                    unsafe { *u2_out = 0; *flag_out = 0 };
                }
                return 0;
            }
            Some(Ok(_)) => {
                any = true;
                let c = one[0];
                if c == b'\n' {
                    if pos > 0 && buf[pos - 1] == b'\r' {
                        pos -= 1;
                    }
                    unsafe { *u2_out = pos as u64; *flag_out = u64::MAX };
                    return 0;
                }
                buf[pos] = c;
                pos += 1;
            }
            _ => {
                unsafe { *u2_out = pos as u64; *flag_out = 0 };
                return (-64i64) as u64;
            }
        }
    }
}

/// delete-file ( c-addr u -- ior )
pub extern "C" fn rt_delete_file(addr: u64, u: u64) -> u64 {
    match std::fs::remove_file(path_from(addr, u)) {
        Ok(()) => 0,
        Err(_) => (-66i64) as u64,
    }
}

/// flush-file ( fid -- ior )
pub extern "C" fn rt_flush_file(fid: u64) -> u64 {
    match with_file(fid, |f| f.flush()) {
        Some(Ok(())) => 0,
        _ => (-71i64) as u64,
    }
}

/// rename-file ( c-addr1 u1 c-addr2 u2 -- ior )
pub extern "C" fn rt_rename_file(a1: u64, u1: u64, a2: u64, u2: u64) -> u64 {
    match std::fs::rename(path_from(a1, u1), path_from(a2, u2)) {
        Ok(()) => 0,
        Err(_) => (-70i64) as u64,
    }
}

// ── Memory-Allocation wordset (ANS): the OS heap via libc malloc/free/realloc ──
/// allocate ( u -- a-addr ior ); kernel passes &addr_out (PAD). ior 0 = success.
pub extern "C" fn rt_allocate(u: u64, addr_out: *mut u64) -> u64 {
    let p = unsafe { libc::malloc(u as usize) };
    if p.is_null() {
        unsafe { *addr_out = 0 };
        (-59i64) as u64 // ALLOCATE failure
    } else {
        unsafe { *addr_out = p as u64 };
        0
    }
}

/// free ( a-addr -- ior )
pub extern "C" fn rt_free(addr: u64) -> u64 {
    if addr != 0 {
        unsafe { libc::free(addr as *mut libc::c_void) };
    }
    0
}

/// resize ( a-addr u -- a-addr' ior ); kernel passes &addr_out. On failure the
/// original block is left intact and addr_out keeps the old address.
pub extern "C" fn rt_resize(addr: u64, u: u64, addr_out: *mut u64) -> u64 {
    let p = unsafe { libc::realloc(addr as *mut libc::c_void, u as usize) };
    if p.is_null() {
        unsafe { *addr_out = addr };
        (-61i64) as u64 // RESIZE failure
    } else {
        unsafe { *addr_out = p as u64 };
        0
    }
}

/// The built-in runtime externs every session binds before assembling the
/// kernel. Names must match the `bl`/`aapcs_call` targets in the kernel.
pub fn externs() -> Vec<(&'static str, *const ())> {
    vec![
        ("rt_double", rt_double as *const ()),
        ("rt_emit", rt_emit as *const ()),
        ("rt_type", rt_type as *const ()),
        ("rt_print_int", rt_print_int as *const ()),
        ("rt_print_float", rt_print_float as *const ()),
        ("rt_fsin", rt_fsin as *const ()),
        ("rt_fcos", rt_fcos as *const ()),
        ("rt_ftan", rt_ftan as *const ()),
        ("rt_fexp", rt_fexp as *const ()),
        ("rt_fln", rt_fln as *const ()),
        ("rt_flog", rt_flog as *const ()),
        ("rt_fatan", rt_fatan as *const ()),
        ("rt_fasin", rt_fasin as *const ()),
        ("rt_facos", rt_facos as *const ()),
        ("rt_fpow", rt_fpow as *const ()),
        ("rt_fatan2", rt_fatan2 as *const ()),
        ("rt_open_file", rt_open_file as *const ()),
        ("rt_create_file", rt_create_file as *const ()),
        ("rt_close_file", rt_close_file as *const ()),
        ("rt_read_file", rt_read_file as *const ()),
        ("rt_write_file", rt_write_file as *const ()),
        ("rt_file_position", rt_file_position as *const ()),
        ("rt_file_size", rt_file_size as *const ()),
        ("rt_reposition_file", rt_reposition_file as *const ()),
        ("rt_write_line", rt_write_line as *const ()),
        ("rt_read_line", rt_read_line as *const ()),
        ("rt_delete_file", rt_delete_file as *const ()),
        ("rt_flush_file", rt_flush_file as *const ()),
        ("rt_rename_file", rt_rename_file as *const ()),
        ("rt_allocate", rt_allocate as *const ()),
        ("rt_free", rt_free as *const ()),
        ("rt_resize", rt_resize as *const ()),
    ]
}
