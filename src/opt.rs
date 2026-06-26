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
    Rot,      // ( a b c -- b c a )
    MinusRot, // ( a b c -- c a b )
    Tuck,     // ( a b -- b a b )
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
/// Floating-point binary ops modeled in the optimizer's FP virtual stack.
#[derive(Clone, Copy, PartialEq)]
pub enum FBin {
    Add,
    Sub,
    Mul,
    Div,
}
/// Floating-point unary ops (single hardware instruction, no libm).
#[derive(Clone, Copy, PartialEq)]
pub enum FUn {
    Neg,
    Sqrt,
    Abs,
}

/// Branchless select words: `min max umin umax` (cmp + csel, foldable).
#[derive(Clone, Copy)]
pub enum Sel {
    Min,
    Max,
    UMin,
    UMax,
}

impl Sel {
    /// Constant-fold `a <sel> b`.
    fn eval(self, a: i64, b: i64) -> i64 {
        match self {
            Sel::Min => a.min(b),
            Sel::Max => a.max(b),
            Sel::UMin => (a as u64).min(b as u64) as i64,
            Sel::UMax => (a as u64).max(b as u64) as i64,
        }
    }
    /// csel condition selecting the NOS operand (`a`) when true.
    fn cond(self) -> u32 {
        match self {
            Sel::Min => LT,  // a < b  → keep a
            Sel::Max => GT,  // a > b  → keep a
            Sel::UMin => LO, // a u< b → keep a
            Sel::UMax => HI, // a u> b → keep a
        }
    }
}

#[derive(Clone, Copy)]
pub enum Tok {
    Lit(i64),
    Bin(Bin),
    ImmBin(Bin, i64), // reduced: TOS op= k
    DupBin(Bin),      // reduced: dup then op (op TOS with itself)
    Stk(Stk),
    Cmp(Cmp),
    Sel(Sel),
    Mem(Mem),
    LocalFetch(u32),  // push local[i] (LP-relative)
    LocalStore(u32),  // local[i] = TOS; drop
    IvarFetch(u32),   // push [SELF + off]  (OOP instance variable)
    IvarStore(u32),   // [SELF + off] = TOS; drop
    SelfPush,         // push the current receiver (user_SELF)
    PickN(u32),       // copy the item at depth N to TOS (0=dup, 1=over, …)
    RollN(u32),       // move the item at depth N to TOS (1=swap, 2=rot, …)
    // Floating-point: modeled in a parallel FP virtual stack (FTOS=d8, FSP).
    FLit(i64),        // float literal (raw bits)
    FBin(FBin),       // f+ / f- / f* / f/
    FUn(FUn),         // fnegate / fsqrt / fabs
    FStk(Stk),        // fdup / fdrop / fswap / fover (FP stack motion)
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

impl Cmp {
    /// Constant-fold a binary comparison: `a <cmp> b` → Forth flag (-1 / 0).
    fn eval(self, a: i64, b: i64) -> i64 {
        let t = match self {
            Cmp::Eq => a == b,
            Cmp::Ne => a != b,
            Cmp::Lt => a < b,
            Cmp::Gt => a > b,
            Cmp::Le => a <= b,
            Cmp::Ge => a >= b,
            Cmp::ULt => (a as u64) < (b as u64),
            Cmp::UGt => (a as u64) > (b as u64),
            _ => return 0, // zero-compares use eval_zero
        };
        if t { -1 } else { 0 }
    }
    /// Constant-fold a zero-compare: `n <0cmp>` → Forth flag.
    fn eval_zero(self, n: i64) -> i64 {
        let t = match self {
            Cmp::ZEq => n == 0,
            Cmp::ZNe => n != 0,
            Cmp::ZLt => n < 0,
            Cmp::ZGt => n > 0,
            _ => return 0,
        };
        if t { -1 } else { 0 }
    }
    /// The csetm condition for a binary compare (`a <cmp> b`).
    fn cond(self) -> u32 {
        match self {
            Cmp::Eq => EQ,
            Cmp::Ne => NE,
            Cmp::Lt => LT,
            Cmp::Gt => GT,
            Cmp::Le => LE,
            Cmp::Ge => GE,
            Cmp::ULt => LO,
            Cmp::UGt => HI,
            _ => EQ,
        }
    }
    /// The comparison whose result is the logical negation of this one (signed
    /// only; unsigned/zero compares have no inverse in the enum). `< 0=` → `>=`.
    fn negate(self) -> Option<Cmp> {
        Some(match self {
            Cmp::Eq => Cmp::Ne,
            Cmp::Ne => Cmp::Eq,
            Cmp::Lt => Cmp::Ge,
            Cmp::Gt => Cmp::Le,
            Cmp::Le => Cmp::Gt,
            Cmp::Ge => Cmp::Lt,
            _ => return None,
        })
    }

    /// The condition with operands swapped (`b <cmp> a`) — for a constant NOS.
    fn swapped_cond(self) -> u32 {
        match self {
            Cmp::Lt => GT,
            Cmp::Gt => LT,
            Cmp::Le => GE,
            Cmp::Ge => LE,
            Cmp::ULt => HI,
            Cmp::UGt => LO,
            other => other.cond(), // Eq/Ne are symmetric
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
            // stack-motion cancellation: inverse permutations annihilate
            Tok::Stk(s) => match (out.last().copied(), s) {
                (Some(Tok::Stk(Stk::Swap)), Stk::Swap)
                | (Some(Tok::Stk(Stk::Rot)), Stk::MinusRot)
                | (Some(Tok::Stk(Stk::MinusRot)), Stk::Rot) => {
                    out.pop();
                }
                _ => out.push(t),
            },
            // logical negation of a comparison: `<cmp> 0=` → the inverse compare.
            Tok::Cmp(Cmp::ZEq) => match out.last().copied() {
                Some(Tok::Cmp(c)) if c.negate().is_some() => {
                    let n = c.negate().unwrap();
                    out.pop();
                    out.push(Tok::Cmp(n));
                }
                _ => out.push(t),
            },
            _ => out.push(t),
        }
    }
    out
}

const TOS: u32 = 0; // canonical top-of-stack register (x0) at window boundaries
const DSP: u32 = 19;
const LP: u32 = 21; // locals-frame pointer
const UP: u32 = 20; // user-area base
const USER_SELF: u32 = 0x1830; // OOP receiver slot in the user area
const POOL: [u32; 7] = [9, 10, 11, 12, 13, 14, 15]; // register window pool


/// A virtual-stack value location: a known constant (not yet in a register) or a
/// pool register. Constants are deferred until consumed; permutations only move
/// `Loc`s around `vs` with no emitted code (WF66's "stack motion is bookkeeping").
#[derive(Clone, Copy, PartialEq)]
enum Loc {
    Const(i64),
    Reg(u32),
}

/// The deferred-assembly lowerer. `vs` is the virtual stack (vs.last() = TOS); a
/// value lives as a `Const` or in a pool register (each register is referenced by
/// at most one `vs` entry — duplications copy). Below `vs` is the entry stack in
/// memory, `consumed` cells of which have been pulled in. Code is emitted only
/// when a value is consumed (arithmetic / memory / call) or forced to canonical
/// memory at a window boundary (`settle`). Stack words are pure `vs` reshuffles.
struct Low<'a> {
    out: &'a mut Vec<u32>,
    vs: Vec<Loc>,
    used: [bool; 32], // which GP registers are live (in vs or a transient)
    consumed: i64,    // entry-memory cells pulled below the window
    // Parallel FP virtual stack (FTOS canonical = d8; pool d9–d15; FSP=x22).
    fvs: Vec<u32>,    // d-registers; fvs.last() = FTOS
    fused: [bool; 32],
    fconsumed: i64,
}

const FTOS: u32 = 8; // d8 — canonical float top of stack at window boundaries
const FSP: u32 = 22;
const FPOOL: [u32; 7] = [9, 10, 11, 12, 13, 14, 15]; // d-register window pool

impl<'a> Low<'a> {
    fn new(out: &'a mut Vec<u32>) -> Self {
        // The incoming canonical form has TOS in x0 and FTOS in d8.
        let mut used = [false; 32];
        used[0] = true;
        let mut fused = [false; 32];
        fused[FTOS as usize] = true;
        Low {
            out,
            vs: vec![Loc::Reg(0)],
            used,
            consumed: 0,
            fvs: vec![FTOS],
            fused,
            fconsumed: 0,
        }
    }

    fn nfree(&self) -> usize {
        POOL.iter().filter(|&&r| !self.used[r as usize]).count()
    }

    fn alloc(&mut self) -> u32 {
        let r = *POOL.iter().find(|&&r| !self.used[r as usize]).expect("alloc: no free reg");
        self.used[r as usize] = true;
        r
    }

    fn freer(&mut self, r: u32) {
        self.used[r as usize] = false;
    }

    fn free_loc(&mut self, l: Loc) {
        if let Loc::Reg(r) = l {
            self.used[r as usize] = false;
        }
    }

    /// Ensure `n` free pool registers, settling the window to memory if not. Call
    /// at the START of an op (while `vs` is consistent) so later allocs never settle.
    fn reserve(&mut self, n: usize) {
        if self.nfree() < n {
            self.settle_data();
        }
    }

    /// Ensure the window holds at least `n` cells, pulling from entry memory.
    fn ensure(&mut self, n: usize) {
        while self.vs.len() < n {
            let r = self.alloc();
            self.out.push(ldr_off(r, DSP, (self.consumed * 8) as u32));
            self.consumed += 1;
            self.vs.insert(0, Loc::Reg(r)); // deeper than the existing window
        }
    }

    /// A value in a register (a `Const` is loaded into a fresh register).
    fn to_reg(&mut self, l: Loc) -> u32 {
        match l {
            Loc::Reg(r) => r,
            Loc::Const(n) => {
                let r = self.alloc();
                load_imm64(r, n as u64, self.out);
                r
            }
        }
    }

    /// A copy of a value (constants are free; registers cost one `mov`).
    fn copy_of(&mut self, l: Loc) -> Loc {
        match l {
            Loc::Const(n) => Loc::Const(n),
            Loc::Reg(r) => {
                let r2 = self.alloc();
                self.out.push(mov_reg(r2, r));
                Loc::Reg(r2)
            }
        }
    }

    fn lit(&mut self, n: i64) {
        self.vs.push(Loc::Const(n)); // deferred — no code
    }

    fn stk(&mut self, s: Stk) {
        match s {
            Stk::Dup => self.pick_n(0),
            Stk::Over => self.pick_n(1),
            Stk::Swap => self.roll_n(1),
            Stk::Rot => self.roll_n(2),
            Stk::Drop => {
                self.ensure(1);
                let t = self.vs.pop().unwrap();
                self.free_loc(t);
            }
            Stk::Nip => {
                self.ensure(2);
                let n = self.vs.len();
                let u = self.vs.remove(n - 2);
                self.free_loc(u);
            }
            Stk::MinusRot => {
                // ( a b c -- c a b )
                self.ensure(3);
                let n = self.vs.len();
                let c = self.vs.pop().unwrap();
                self.vs.insert(n - 3, c);
            }
            Stk::Tuck => {
                // ( a b -- b a b )
                self.reserve(3);
                self.ensure(2);
                let n = self.vs.len();
                let top = self.vs[n - 1];
                let c = self.copy_of(top);
                self.vs.insert(n - 2, c);
            }
        }
    }

    /// `N pick` — copy the item at depth N to TOS (0 = dup, 1 = over).
    fn pick_n(&mut self, n: u32) {
        let n = n as usize;
        self.reserve(n + 2);
        self.ensure(n + 1);
        let src = self.vs[self.vs.len() - 1 - n];
        let c = self.copy_of(src);
        self.vs.push(c);
    }

    /// `N roll` — move the item at depth N to TOS (1 = swap, 2 = rot).
    fn roll_n(&mut self, n: u32) {
        if n == 0 {
            return; // identity
        }
        let n = n as usize;
        self.reserve(n + 1);
        self.ensure(n + 1);
        let idx = self.vs.len() - 1 - n;
        let item = self.vs.remove(idx);
        self.vs.push(item); // pure move — no code
    }

    /// `add_imm`/`sub_imm` of a single operand by constant `k` (Add/Sub only).
    fn imm_bin_to(&mut self, operand: Loc, op: Bin, k: i64) -> Option<u32> {
        let (add, imm) = match op {
            Bin::Add if (0..=4095).contains(&k) => (true, k as u32),
            Bin::Add if (-4095..0).contains(&k) => (false, (-k) as u32),
            Bin::Sub if (0..=4095).contains(&k) => (false, k as u32),
            Bin::Sub if (-4095..0).contains(&k) => (true, (-k) as u32),
            _ => return None,
        };
        let r = self.to_reg(operand);
        self.out.push(if add { add_imm(r, r, imm) } else { sub_imm(r, r, imm) });
        Some(r)
    }

    fn bin(&mut self, op: Bin) {
        self.reserve(4);
        self.ensure(2);
        let b = self.vs.pop().unwrap(); // TOS
        let a = self.vs.pop().unwrap(); // NOS
        // const-fold
        if let (Loc::Const(x), Loc::Const(y)) = (a, b) {
            self.vs.push(Loc::Const(op.eval(x, y)));
            return;
        }
        // immediate-fold: result = a <op> b
        if let Loc::Const(k) = b {
            if let Some(rd) = self.imm_bin_to(a, op, k) {
                self.vs.push(Loc::Reg(rd));
                return;
            }
        }
        if let Loc::Const(k) = a {
            if op == Bin::Add {
                if let Some(rd) = self.imm_bin_to(b, Bin::Add, k) {
                    self.vs.push(Loc::Reg(rd));
                    return;
                }
            }
        }
        // general: a <op> b into a register (reuse a's)
        let ra = self.to_reg(a);
        let rb = self.to_reg(b);
        self.out.push(match op {
            Bin::Add => add_reg(ra, ra, rb),
            Bin::Sub => sub_reg(ra, ra, rb), // a - b
            Bin::Mul => mul(ra, ra, rb),
            Bin::And => and_reg(ra, ra, rb),
            Bin::Or => orr_reg(ra, ra, rb),
            Bin::Xor => eor_reg(ra, ra, rb),
        });
        self.freer(rb); // b consumed (rb != ra: no aliasing)
        self.vs.push(Loc::Reg(ra));
    }

    /// `TOS op= k` (reduced `Lit k, Bin`).
    fn imm_bin(&mut self, op: Bin, k: i64) {
        self.reserve(2);
        self.ensure(1);
        let a = self.vs.pop().unwrap();
        if let Loc::Const(x) = a {
            self.vs.push(Loc::Const(op.eval(x, k)));
            return;
        }
        if let Some(rd) = self.imm_bin_to(a, op, k) {
            self.vs.push(Loc::Reg(rd));
            return;
        }
        let r = self.to_reg(a);
        let t = self.alloc();
        load_imm64(t, k as u64, self.out);
        self.out.push(match op {
            Bin::Add => add_reg(r, r, t),
            Bin::Sub => sub_reg(r, r, t),
            Bin::Mul => mul(r, r, t),
            Bin::And => and_reg(r, r, t),
            Bin::Or => orr_reg(r, r, t),
            Bin::Xor => eor_reg(r, r, t),
        });
        self.freer(t);
        self.vs.push(Loc::Reg(r));
    }

    /// `dup` then `op` — the value combined with itself (reduced `Dup, Bin`).
    fn dup_bin(&mut self, op: Bin) {
        self.reserve(2);
        self.ensure(1);
        let a = self.vs.pop().unwrap();
        if let Loc::Const(x) = a {
            self.vs.push(Loc::Const(op.eval(x, x)));
            return;
        }
        match op {
            Bin::Add => {
                let r = self.to_reg(a);
                self.out.push(add_reg(r, r, r)); // 2a
                self.vs.push(Loc::Reg(r));
            }
            Bin::Mul => {
                let r = self.to_reg(a);
                self.out.push(mul(r, r, r)); // a*a
                self.vs.push(Loc::Reg(r));
            }
            Bin::And | Bin::Or => self.vs.push(a), // a&a = a|a = a
            Bin::Sub | Bin::Xor => {
                self.free_loc(a);
                self.vs.push(Loc::Const(0)); // a-a = a^a = 0
            }
        }
    }

    fn cmp(&mut self, c: Cmp) {
        self.reserve(4);
        match c {
            Cmp::ZEq | Cmp::ZNe | Cmp::ZLt | Cmp::ZGt => {
                self.ensure(1);
                let a = self.vs.pop().unwrap();
                if let Loc::Const(n) = a {
                    self.vs.push(Loc::Const(c.eval_zero(n))); // fold `n 0=` etc.
                    return;
                }
                let r = self.to_reg(a);
                let cond = match c {
                    Cmp::ZEq => EQ,
                    Cmp::ZNe => NE,
                    Cmp::ZLt => LT,
                    _ => GT,
                };
                self.out.push(cmp_imm(r, 0));
                self.out.push(csetm(r, cond));
                self.vs.push(Loc::Reg(r));
            }
            _ => {
                self.ensure(2);
                let b = self.vs.pop().unwrap();
                let a = self.vs.pop().unwrap();
                // const-fold a fully-constant comparison
                if let (Loc::Const(x), Loc::Const(y)) = (a, b) {
                    self.vs.push(Loc::Const(c.eval(x, y)));
                    return;
                }
                // immediate compare against a constant TOS: `a <cmp> k` → cmp ra,#k
                if let Loc::Const(k) = b {
                    if (0..=4095).contains(&k) {
                        let ra = self.to_reg(a);
                        self.out.push(cmp_imm(ra, k as u32));
                        self.out.push(csetm(ra, c.cond()));
                        self.vs.push(Loc::Reg(ra));
                        return;
                    }
                }
                // immediate compare against a constant NOS: `k <cmp> b` → cmp rb,#k
                // with the condition swapped (b on the left).
                if let Loc::Const(k) = a {
                    if (0..=4095).contains(&k) {
                        let rb = self.to_reg(b);
                        self.out.push(cmp_imm(rb, k as u32));
                        self.out.push(csetm(rb, c.swapped_cond()));
                        self.vs.push(Loc::Reg(rb));
                        return;
                    }
                }
                let ra = self.to_reg(a);
                let rb = self.to_reg(b);
                let (rn, rm) = match c {
                    Cmp::Eq | Cmp::Ne => (ra, rb), // symmetric
                    _ => (ra, rb),                 // a <cmp> b
                };
                self.out.push(cmp_reg(rn, rm));
                self.out.push(csetm(ra, c.cond()));
                self.freer(rb);
                self.vs.push(Loc::Reg(ra));
            }
        }
    }

    /// min / max / umin / umax — branchless via cmp + csel, const-folded.
    fn sel(&mut self, s: Sel) {
        self.reserve(3);
        self.ensure(2);
        let b = self.vs.pop().unwrap();
        let a = self.vs.pop().unwrap();
        if let (Loc::Const(x), Loc::Const(y)) = (a, b) {
            self.vs.push(Loc::Const(s.eval(x, y)));
            return;
        }
        let ra = self.to_reg(a);
        let rb = self.to_reg(b);
        self.out.push(cmp_reg(ra, rb)); // a ? b
        self.out.push(csel(ra, ra, rb, s.cond())); // keep a when cond, else b
        self.freer(rb);
        self.vs.push(Loc::Reg(ra));
    }

    fn mem(&mut self, m: Mem) {
        self.reserve(3);
        match m {
            Mem::Fetch => {
                self.ensure(1);
                let a = self.vs.pop().unwrap();
                let r = self.to_reg(a);
                self.out.push(ldr0(r, r));
                self.vs.push(Loc::Reg(r));
            }
            Mem::CFetch => {
                self.ensure(1);
                let a = self.vs.pop().unwrap();
                let r = self.to_reg(a);
                self.out.push(ldrb0(r, r));
                self.vs.push(Loc::Reg(r));
            }
            Mem::Store => {
                // ( x addr -- )
                self.ensure(2);
                let addr = self.vs.pop().unwrap();
                let x = self.vs.pop().unwrap();
                let raddr = self.to_reg(addr);
                let rx = self.to_reg(x);
                self.out.push(str_off(rx, raddr, 0));
                self.freer(rx);
                self.freer(raddr);
            }
            Mem::CStore => {
                self.ensure(2);
                let addr = self.vs.pop().unwrap();
                let x = self.vs.pop().unwrap();
                let raddr = self.to_reg(addr);
                let rx = self.to_reg(x);
                self.out.push(strb0(rx, raddr));
                self.freer(rx);
                self.freer(raddr);
            }
        }
    }

    fn local_fetch(&mut self, i: u32) {
        self.reserve(1);
        let r = self.alloc();
        self.out.push(ldr_off(r, LP, i * 8));
        self.vs.push(Loc::Reg(r));
    }

    fn local_store(&mut self, i: u32) {
        self.reserve(2);
        self.ensure(1);
        let a = self.vs.pop().unwrap();
        let r = self.to_reg(a);
        self.out.push(str_off(r, LP, i * 8));
        self.freer(r);
    }

    fn ivar_fetch(&mut self, off: u32) {
        self.reserve(1);
        let r = self.alloc();
        self.out.push(ldr_off(r, UP, USER_SELF)); // r = SELF
        self.out.push(ldr_off(r, r, off)); // r = [SELF + off]
        self.vs.push(Loc::Reg(r));
    }

    fn ivar_store(&mut self, off: u32) {
        self.reserve(2);
        self.ensure(1);
        let a = self.vs.pop().unwrap();
        let rv = self.to_reg(a);
        let rs = self.alloc();
        self.out.push(ldr_off(rs, UP, USER_SELF));
        self.out.push(str_off(rv, rs, off));
        self.freer(rv);
        self.freer(rs);
    }

    fn self_push(&mut self) {
        self.reserve(1);
        let r = self.alloc();
        self.out.push(ldr_off(r, UP, USER_SELF));
        self.vs.push(Loc::Reg(r));
    }

    // ── Floating-point virtual stack (parallel to the data stack) ────────────
    fn ffree(&self) -> usize {
        FPOOL.iter().filter(|&&r| !self.fused[r as usize]).count()
    }
    fn falloc(&mut self) -> u32 {
        if self.ffree() == 0 {
            self.fsettle();
        }
        let r = *FPOOL.iter().find(|&&r| !self.fused[r as usize]).expect("falloc");
        self.fused[r as usize] = true;
        r
    }
    /// A free GP register for materializing a float constant (settling the data
    /// window if the GP pool is full — FP code rarely holds a deep data window).
    fn gp_temp(&mut self) -> u32 {
        if self.nfree() == 0 {
            self.settle_data();
        }
        let r = *POOL.iter().find(|&&r| !self.used[r as usize]).expect("gp_temp");
        self.used[r as usize] = true;
        r
    }
    fn fensure(&mut self, n: usize) {
        while self.fvs.len() < n {
            let r = self.falloc();
            self.out.push(fldr_off(r, FSP, (self.fconsumed * 8) as u32));
            self.fconsumed += 1;
            self.fvs.insert(0, r);
        }
    }
    fn flit(&mut self, bits: i64) {
        let g = self.gp_temp();
        load_imm64(g, bits as u64, self.out);
        let d = self.falloc();
        self.out.push(fmov_dx(d, g)); // d = bits reinterpreted as double
        self.used[g as usize] = false;
        self.fvs.push(d);
    }
    fn fbin(&mut self, op: FBin) {
        self.fensure(2);
        let b = self.fvs.pop().unwrap();
        let a = self.fvs.pop().unwrap();
        self.out.push(match op {
            FBin::Add => fadd(a, a, b),
            FBin::Sub => fsub(a, a, b), // a - b
            FBin::Mul => fmul(a, a, b),
            FBin::Div => fdiv(a, a, b), // a / b
        });
        self.fused[b as usize] = false;
        self.fvs.push(a);
    }
    fn fun(&mut self, op: FUn) {
        self.fensure(1);
        let a = *self.fvs.last().unwrap();
        self.out.push(match op {
            FUn::Neg => fneg(a, a),
            FUn::Sqrt => fsqrt(a, a),
            FUn::Abs => fabs(a, a),
        });
    }
    fn fstk(&mut self, s: Stk) {
        match s {
            Stk::Dup => {
                self.fensure(1);
                let top = *self.fvs.last().unwrap();
                let d = self.falloc();
                self.out.push(fmov_dd(d, top));
                self.fvs.push(d);
            }
            Stk::Drop => {
                self.fensure(1);
                let r = self.fvs.pop().unwrap();
                self.fused[r as usize] = false;
            }
            Stk::Swap => {
                self.fensure(2);
                let n = self.fvs.len();
                self.fvs.swap(n - 1, n - 2);
            }
            Stk::Over => {
                self.fensure(2);
                let n = self.fvs.len();
                let u = self.fvs[n - 2];
                let d = self.falloc();
                self.out.push(fmov_dd(d, u));
                self.fvs.push(d);
            }
            _ => {} // other motions unused for FP
        }
    }
    /// Write the FP virtual stack back to canonical form (FTOS in d8, the rest in
    /// memory from FSP) and reset.
    fn fsettle(&mut self) {
        if self.fvs.is_empty() {
            self.fensure(1);
        }
        let l = self.fvs.len();
        let delta = self.fconsumed - (l as i64 - 1);
        if delta > 0 {
            self.out.push(add_imm(FSP, FSP, (delta * 8) as u32));
        } else if delta < 0 {
            self.out.push(sub_imm(FSP, FSP, ((-delta) * 8) as u32));
        }
        for i in 0..l - 1 {
            let off = ((l - 2 - i) as u32) * 8;
            self.out.push(fstr_off(self.fvs[i], FSP, off));
            self.fused[self.fvs[i] as usize] = false;
        }
        let top = self.fvs[l - 1];
        if top != FTOS {
            self.out.push(fmov_dd(FTOS, top));
            self.fused[top as usize] = false;
        }
        self.fvs.clear();
        self.fvs.push(FTOS);
        self.fused = [false; 32];
        self.fused[FTOS as usize] = true;
        self.fconsumed = 0;
    }

    /// Settle both the data and FP windows to canonical form (before a Call and at
    /// the run's end).
    fn settle(&mut self) {
        self.settle_data();
        self.fsettle();
    }

    /// Write the data virtual stack back to the canonical form (TOS in x0, the rest
    /// in memory from DSP) and reset.
    fn settle_data(&mut self) {
        if self.vs.is_empty() {
            self.ensure(1); // materialize a TOS from entry memory
        }
        let l = self.vs.len();
        let delta = self.consumed - (l as i64 - 1); // DSP change in cells
        if delta > 0 {
            self.out.push(add_imm(DSP, DSP, (delta * 8) as u32));
        } else if delta < 0 {
            self.out.push(sub_imm(DSP, DSP, ((-delta) * 8) as u32));
        }
        // Pass 1: store deep register entries (freeing their regs), reading x0
        // before it is overwritten by the new TOS.
        for i in 0..l - 1 {
            if let Loc::Reg(r) = self.vs[i] {
                let off = ((l - 2 - i) as u32) * 8;
                self.out.push(str_off(r, DSP, off));
                self.freer(r);
            }
        }
        // The new TOS goes to x0.
        match self.vs[l - 1] {
            Loc::Reg(0) => {}
            Loc::Reg(r) => {
                self.out.push(mov_reg(TOS, r));
                self.freer(r);
            }
            Loc::Const(n) => load_imm64(TOS, n as u64, self.out),
        }
        // Pass 2: store deep constant entries via a now-free scratch register.
        for i in 0..l - 1 {
            if let Loc::Const(n) = self.vs[i] {
                let off = ((l - 2 - i) as u32) * 8;
                let t = self.alloc();
                load_imm64(t, n as u64, self.out);
                self.out.push(str_off(t, DSP, off));
                self.freer(t);
            }
        }
        self.vs.clear();
        self.vs.push(Loc::Reg(0));
        self.used = [false; 32];
        self.used[0] = true;
        self.consumed = 0;
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

/// Lower one straight-line token run to AArch64 with the deferred virtual-stack
/// model (appended to `out`). Constants and stack motion stay virtual; code is
/// emitted only at consume/settle. Settles before each call and at the end so the
/// canonical TOS=x0 / rest-in-memory form holds at every window boundary.
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
            Tok::Sel(s) => low.sel(s),
            Tok::Mem(m) => low.mem(m),
            Tok::LocalFetch(i) => low.local_fetch(i),
            Tok::LocalStore(i) => low.local_store(i),
            Tok::IvarFetch(off) => low.ivar_fetch(off),
            Tok::IvarStore(off) => low.ivar_store(off),
            Tok::SelfPush => low.self_push(),
            Tok::PickN(n) => low.pick_n(n),
            Tok::RollN(n) => low.roll_n(n),
            Tok::FLit(bits) => low.flit(bits),
            Tok::FBin(op) => low.fbin(op),
            Tok::FUn(op) => low.fun(op),
            Tok::FStk(s) => low.fstk(s),
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

