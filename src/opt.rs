//! The MF66 token-IR optimizer (the "optimizing" half of the compiler).
//!
//! A colon body's straight-line runs are accumulated as `Tok`s, `reduce`d
//! (const-fold, immediate-fold, dup-fuse, dead-code elimination), then `lower`ed
//! to tight AArch64 — replacing the eager veneer-call-per-word with register code.
//! Control-flow directives flush the current run, so optimization covers the
//! straight-line segments between branches (the bulk of real code).

use crate::aenc::*;

#[derive(Clone, Copy, PartialEq)]
pub enum Bin {
    Add,
    Sub,
    Mul,
    And,
    Or,
    Xor,
}
#[derive(Clone, Copy, PartialEq)]
pub enum Stk {
    Dup,
    Drop,
    Swap,
    Over,
    Nip,
}
#[derive(Clone, Copy, PartialEq)]
pub enum Cmp {
    Eq,
    Ne,
    Lt,
    Gt,
    Le,
    Ge,
    ULt,
    UGt,
    ZEq,
    ZNe,
    ZLt,
    ZGt,
}
#[derive(Clone, Copy, PartialEq)]
pub enum Mem {
    Fetch,
    Store,
    CFetch,
    CStore,
}

#[derive(Clone, Copy)]
pub enum Tok {
    Lit(i64),
    Bin(Bin),
    ImmBin(Bin, i64), // reduced: TOS op= k
    DupBin(Bin),      // reduced: dup then op (op TOS with itself)
    Stk(Stk),
    Cmp(Cmp),
    Mem(Mem),
    LocalFetch(u32),  // push local[i] (LP-relative)
    LocalStore(u32),  // local[i] = TOS; drop
    IvarFetch(u32),   // push [SELF + off]  (OOP instance variable)
    IvarStore(u32),   // [SELF + off] = TOS; drop
    SelfPush,         // push the current receiver (user_SELF)
    Call(u64),
}

impl Bin {
    /// `a op b` where a = NOS (deeper), b = TOS.
    fn eval(self, a: i64, b: i64) -> i64 {
        match self {
            Bin::Add => a.wrapping_add(b),
            Bin::Sub => a.wrapping_sub(b),
            Bin::Mul => a.wrapping_mul(b),
            Bin::And => a & b,
            Bin::Or => a | b,
            Bin::Xor => a ^ b,
        }
    }
}

/// Reduce a token run: const-fold `Lit Lit Bin`, immediate-fold `Lit Bin`,
/// dup-fuse `Dup Bin`, and DCE `Lit Drop` / `Dup Drop`. A single forward pass
/// with lookback on the output is a fixpoint for these linear stack rewrites.
pub fn reduce(toks: &[Tok]) -> Vec<Tok> {
    let mut out: Vec<Tok> = Vec::with_capacity(toks.len());
    for &t in toks {
        match t {
            Tok::Bin(op) => {
                let n = out.len();
                // const-fold: ... Lit a, Lit b, Bin -> Lit (a op b)
                if n >= 2 {
                    if let (Tok::Lit(a), Tok::Lit(b)) = (out[n - 2], out[n - 1]) {
                        out.truncate(n - 2);
                        out.push(Tok::Lit(op.eval(a, b)));
                        continue;
                    }
                }
                // immediate-fold: ... Lit k, Bin -> ImmBin(op, k)
                if let Some(Tok::Lit(k)) = out.last().copied() {
                    out.pop();
                    out.push(Tok::ImmBin(op, k));
                    continue;
                }
                // dup-fuse: ... Dup, Bin -> DupBin(op)
                if let Some(Tok::Stk(Stk::Dup)) = out.last().copied() {
                    out.pop();
                    out.push(Tok::DupBin(op));
                    continue;
                }
                out.push(t);
            }
            Tok::Stk(Stk::Drop) => match out.last().copied() {
                // DCE: a literal or a dup immediately dropped is dead
                Some(Tok::Lit(_)) | Some(Tok::Stk(Stk::Dup)) => {
                    out.pop();
                }
                _ => out.push(t),
            },
            _ => out.push(t),
        }
    }
    out
}

const TOS: u32 = 0; // x0 — always holds the top of stack within a window
const DSP: u32 = 19;
const LP: u32 = 21; // locals-frame pointer
const UP: u32 = 20; // user-area base
const USER_SELF: u32 = 0x1830; // OOP receiver slot in the user area
const POOL: [u32; 7] = [9, 10, 11, 12, 13, 14, 15]; // register window below TOS

/// Register-windowing lowerer (O2). The invariant: TOS is in x0; the next cells
/// down live in `below` registers (below[0] = NOS); cells beyond the window are in
/// memory. Memory is touched lazily — only when a value must be pulled in, when
/// the pool overflows, or at a window boundary (`settle`: before a Call and at the
/// end). This replaces O1's per-op ldr/str with register-resident stack cells.
struct Low<'a> {
    out: &'a mut Vec<u32>,
    below: Vec<u32>, // registers below TOS; below[0] = NOS
    consumed: i64,   // memory cells pulled from below the window (cells)
}

impl<'a> Low<'a> {
    fn new(out: &'a mut Vec<u32>) -> Self {
        Low { out, below: Vec::new(), consumed: 0 }
    }

    /// A pool register not currently in the window (settling first if all are in use).
    fn alloc(&mut self) -> u32 {
        if let Some(&r) = POOL.iter().find(|r| !self.below.contains(r)) {
            return r;
        }
        self.settle();
        POOL[0]
    }

    /// Remove NOS (below[0]) into a register, pulling it from memory if the window
    /// is empty. The returned register is no longer part of the window.
    fn pop_nos(&mut self) -> u32 {
        if !self.below.is_empty() {
            return self.below.remove(0);
        }
        let r = self.alloc();
        self.out.push(ldr_off(r, DSP, (self.consumed * 8) as u32));
        self.consumed += 1;
        r
    }

    /// Ensure NOS is resident in the window and return its register (kept in place).
    fn ensure_nos(&mut self) -> u32 {
        if self.below.is_empty() {
            let r = self.alloc();
            self.out.push(ldr_off(r, DSP, (self.consumed * 8) as u32));
            self.consumed += 1;
            self.below.insert(0, r);
        }
        self.below[0]
    }

    /// Push the current TOS down into the window (caller then sets the new TOS).
    fn push_down(&mut self) {
        let r = self.alloc();
        self.out.push(mov_reg(r, TOS));
        self.below.insert(0, r);
    }

    /// A transient scratch register (not tracked in the window).
    fn scratch(&mut self) -> u32 {
        self.alloc()
    }

    /// Settle the window if fewer than `need` pool registers are free, so an op
    /// that holds window cells across an `alloc` won't have them spilled mid-op.
    fn settle_if_tight(&mut self, need: usize) {
        if POOL.len() - self.below.len() < need {
            self.settle();
        }
    }

    /// Write the window back to memory in canonical form (TOS stays in x0); reset.
    fn settle(&mut self) {
        let l = self.below.len() as i64;
        let delta = self.consumed - l; // net DSP change, in cells
        if delta > 0 {
            self.out.push(add_imm(DSP, DSP, (delta * 8) as u32));
        } else if delta < 0 {
            self.out.push(sub_imm(DSP, DSP, ((-delta) * 8) as u32));
        }
        for i in 0..self.below.len() {
            self.out.push(str_off(self.below[i], DSP, (i as u32) * 8)); // below[0]=NOS@[DSP]
        }
        self.below.clear();
        self.consumed = 0;
    }

    fn lit(&mut self, n: i64) {
        self.push_down();
        load_imm64(TOS, n as u64, self.out);
    }

    fn bin(&mut self, op: Bin) {
        let r = self.pop_nos();
        self.out.push(match op {
            Bin::Add => add_reg(TOS, TOS, r),
            Bin::Sub => sub_reg(TOS, r, TOS), // NOS - TOS
            Bin::Mul => mul(TOS, TOS, r),
            Bin::And => and_reg(TOS, TOS, r),
            Bin::Or => orr_reg(TOS, TOS, r),
            Bin::Xor => eor_reg(TOS, TOS, r),
        });
    }

    fn imm_bin(&mut self, op: Bin, k: i64) {
        let small = (0..=4095).contains(&k);
        let nsmall = (-4095..0).contains(&k);
        match op {
            Bin::Add if small => self.out.push(add_imm(TOS, TOS, k as u32)),
            Bin::Add if nsmall => self.out.push(sub_imm(TOS, TOS, (-k) as u32)),
            Bin::Sub if small => self.out.push(sub_imm(TOS, TOS, k as u32)),
            Bin::Sub if nsmall => self.out.push(add_imm(TOS, TOS, (-k) as u32)),
            _ => {
                let r = self.scratch();
                load_imm64(r, k as u64, self.out);
                self.out.push(match op {
                    Bin::Add => add_reg(TOS, TOS, r),
                    Bin::Sub => sub_reg(TOS, TOS, r), // TOS - k
                    Bin::Mul => mul(TOS, TOS, r),
                    Bin::And => and_reg(TOS, TOS, r),
                    Bin::Or => orr_reg(TOS, TOS, r),
                    Bin::Xor => eor_reg(TOS, TOS, r),
                });
            }
        }
    }

    fn dup_bin(&mut self, op: Bin) {
        match op {
            Bin::Add => self.out.push(add_reg(TOS, TOS, TOS)),
            Bin::Mul => self.out.push(mul(TOS, TOS, TOS)),
            Bin::And | Bin::Or => {} // a&a = a|a = a
            Bin::Sub | Bin::Xor => self.out.push(movz(TOS, 0, 0)), // 0
        }
    }

    fn stk(&mut self, s: Stk) {
        match s {
            Stk::Dup => {
                let r = self.alloc();
                self.out.push(mov_reg(r, TOS));
                self.below.insert(0, r);
            }
            Stk::Drop => {
                let r = self.pop_nos();
                self.out.push(mov_reg(TOS, r));
            }
            Stk::Swap => {
                self.settle_if_tight(2); // need NOS + a distinct scratch
                let n = self.ensure_nos();
                let t = self.scratch(); // ≠ n (scratch avoids window regs)
                self.out.push(mov_reg(t, TOS));
                self.out.push(mov_reg(TOS, n));
                self.out.push(mov_reg(n, t));
            }
            Stk::Over => {
                self.settle_if_tight(2); // need NOS kept + a push-down reg
                let _ = self.ensure_nos(); // a = below[0]
                self.push_down(); // b -> below[0]; a now at below[1]
                let a = self.below[1];
                self.out.push(mov_reg(TOS, a));
            }
            Stk::Nip => {
                if self.below.is_empty() {
                    self.consumed += 1; // skip the memory NOS
                } else {
                    self.below.remove(0);
                }
            }
        }
    }

    fn cmp(&mut self, c: Cmp) {
        match c {
            Cmp::ZEq => {
                self.out.push(cmp_imm(TOS, 0));
                self.out.push(csetm(TOS, EQ));
            }
            Cmp::ZNe => {
                self.out.push(cmp_imm(TOS, 0));
                self.out.push(csetm(TOS, NE));
            }
            Cmp::ZLt => {
                self.out.push(cmp_imm(TOS, 0));
                self.out.push(csetm(TOS, LT));
            }
            Cmp::ZGt => {
                self.out.push(cmp_imm(TOS, 0));
                self.out.push(csetm(TOS, GT));
            }
            _ => {
                let n = self.pop_nos();
                let (rn, rm, cond) = match c {
                    Cmp::Eq => (TOS, n, EQ),
                    Cmp::Ne => (TOS, n, NE),
                    Cmp::Lt => (n, TOS, LT), // NOS < TOS
                    Cmp::Gt => (n, TOS, GT),
                    Cmp::Le => (n, TOS, LE),
                    Cmp::Ge => (n, TOS, GE),
                    Cmp::ULt => (n, TOS, LO),
                    Cmp::UGt => (n, TOS, HI),
                    _ => unreachable!(),
                };
                self.out.push(cmp_reg(rn, rm));
                self.out.push(csetm(TOS, cond));
            }
        }
    }

    fn local_fetch(&mut self, i: u32) {
        self.push_down();
        self.out.push(ldr_off(TOS, LP, i * 8));
    }

    fn local_store(&mut self, i: u32) {
        self.out.push(str_off(TOS, LP, i * 8)); // local[i] = TOS
        let r = self.pop_nos(); // drop TOS, raise NOS
        self.out.push(mov_reg(TOS, r));
    }

    fn self_push(&mut self) {
        self.push_down();
        self.out.push(ldr_off(TOS, UP, USER_SELF)); // TOS = [UP + user_SELF]
    }

    fn ivar_fetch(&mut self, off: u32) {
        self.push_down();
        let r = self.alloc(); // transient: the SELF base
        self.out.push(ldr_off(r, UP, USER_SELF));
        self.out.push(ldr_off(TOS, r, off)); // TOS = [SELF + off]
    }

    fn ivar_store(&mut self, off: u32) {
        let r = self.scratch(); // SELF base (transient)
        self.out.push(ldr_off(r, UP, USER_SELF));
        self.out.push(str_off(TOS, r, off)); // [SELF + off] = TOS
        let v = self.pop_nos(); // drop TOS, raise NOS
        self.out.push(mov_reg(TOS, v));
    }

    fn mem(&mut self, m: Mem) {
        match m {
            Mem::Fetch => self.out.push(ldr0(TOS, TOS)),
            Mem::CFetch => self.out.push(ldrb0(TOS, TOS)),
            Mem::Store => {
                let v = self.pop_nos();
                self.out.push(str_off(v, TOS, 0)); // [addr] = value
                let nt = self.pop_nos();
                self.out.push(mov_reg(TOS, nt)); // drop addr → new TOS
            }
            Mem::CStore => {
                let v = self.pop_nos();
                self.out.push(strb0(v, TOS));
                let nt = self.pop_nos();
                self.out.push(mov_reg(TOS, nt));
            }
        }
    }
}

/// Fused comparison for a `Cmp` immediately followed by `if`/`until`/`while`:
/// consumes the operand(s) (canonical stack: TOS=x0, NOS=[DSP]), sets the CPU
/// flags, and returns the condition code that holds iff the Forth comparison is
/// TRUE — no `-1/0` flag is materialized. The caller branches on the inverse
/// (false → branch). Assumes the rest of the run was already settled.
pub fn fused_cmp(c: Cmp, out: &mut Vec<u32>) -> u32 {
    const T: u32 = 9;
    match c {
        Cmp::ZEq | Cmp::ZNe | Cmp::ZLt | Cmp::ZGt => {
            out.push(cmp_imm(TOS, 0));
            out.push(ldr_post(TOS, DSP, 8)); // drop n, raise NOS into TOS
            match c {
                Cmp::ZEq => EQ,
                Cmp::ZNe => NE,
                Cmp::ZLt => LT,
                Cmp::ZGt => GT,
                _ => unreachable!(),
            }
        }
        _ => {
            out.push(ldr_post(T, DSP, 8)); // x9 = NOS (a)
            let (rn, rm, ctrue) = match c {
                Cmp::Eq => (TOS, T, EQ),
                Cmp::Ne => (TOS, T, NE),
                Cmp::Lt => (T, TOS, LT), // a < b
                Cmp::Gt => (T, TOS, GT),
                Cmp::Le => (T, TOS, LE),
                Cmp::Ge => (T, TOS, GE),
                Cmp::ULt => (T, TOS, LO),
                Cmp::UGt => (T, TOS, HI),
                _ => unreachable!(),
            };
            out.push(cmp_reg(rn, rm));
            out.push(ldr_post(TOS, DSP, 8)); // drop b, raise next into TOS
            ctrue
        }
    }
}

/// Lower one reduced token run to AArch64 with register windowing (appended to
/// `out`). Settles before each call and at the end so the canonical TOS=x0 /
/// rest-in-memory form holds at every window boundary.
pub fn lower(toks: &[Tok], out: &mut Vec<u32>) {
    let mut low = Low::new(out);
    for &t in toks {
        match t {
            Tok::Lit(n) => low.lit(n),
            Tok::Bin(op) => low.bin(op),
            Tok::ImmBin(op, k) => low.imm_bin(op, k),
            Tok::DupBin(op) => low.dup_bin(op),
            Tok::Stk(s) => low.stk(s),
            Tok::Cmp(c) => low.cmp(c),
            Tok::Mem(m) => low.mem(m),
            Tok::LocalFetch(i) => low.local_fetch(i),
            Tok::LocalStore(i) => low.local_store(i),
            Tok::IvarFetch(off) => low.ivar_fetch(off),
            Tok::IvarStore(off) => low.ivar_store(off),
            Tok::SelfPush => low.self_push(),
            Tok::Call(xt) => {
                low.settle();
                emit_call(xt, low.out);
            }
        }
    }
    low.settle();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn const_fold() {
        // 2 3 + -> Lit 5
        let r = reduce(&[Tok::Lit(2), Tok::Lit(3), Tok::Bin(Bin::Add)]);
        assert_eq!(r.len(), 1);
        assert!(matches!(r[0], Tok::Lit(5)));
        // 2 3 + 4 * -> Lit 20
        let r = reduce(&[
            Tok::Lit(2),
            Tok::Lit(3),
            Tok::Bin(Bin::Add),
            Tok::Lit(4),
            Tok::Bin(Bin::Mul),
        ]);
        assert!(matches!(r[..], [Tok::Lit(20)]));
    }

    #[test]
    fn imm_and_dup_and_dce() {
        // x 5 + -> ImmBin(Add,5)
        let r = reduce(&[Tok::Bin(Bin::Add), Tok::Lit(5), Tok::Bin(Bin::Add)]);
        assert!(matches!(r.last(), Some(Tok::ImmBin(Bin::Add, 5))));
        // dup + -> DupBin(Add)
        let r = reduce(&[Tok::Stk(Stk::Dup), Tok::Bin(Bin::Add)]);
        assert!(matches!(r[..], [Tok::DupBin(Bin::Add)]));
        // 7 drop -> nothing
        let r = reduce(&[Tok::Lit(7), Tok::Stk(Stk::Drop)]);
        assert!(r.is_empty());
    }
}
