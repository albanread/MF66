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
    // Locals-frame open/close as balanced body tokens, so a locals word inlines
    // into a caller as a correct NESTED sub-frame (LP dips and pops). Only the
    // inline IR carries these; standalone bodies set up / tear down the frame
    // directly (open_locals + the commit_body epilogue).
    OpenLocals { total: u32, inputs: u32 }, // sub LP; pop `inputs` from data stack
    CloseLocals { total: u32 },             // add LP (frame freed)
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

/// Per-word optimizer metrics, accumulated across a definition's straight-line
/// runs (each control-flow boundary flushes a run through reduce + lower). The
/// counters mirror the optimizer's levers, grouped into token reduction,
/// register/stack handling, and memory traffic. See `Mf66Session::optimizer_report`.
#[derive(Clone, Default, Debug)]
pub struct Metrics {
    // ── reduce: token-IR shrinkage ──────────────────────────────────────────
    pub toks_in: u32,       // tokens captured (pre-reduce)
    pub toks_out: u32,      // tokens after reduce
    pub const_folds: u32,   // Lit Lit Bin → Lit (+ Cmp/Sel folds in the lowerer)
    pub imm_folds: u32,     // Lit Bin → ImmBin
    pub imm_chains: u32,    // ImmBin ImmBin → ImmBin (e.g. 1+ 1+ → +2)
    pub dup_fuses: u32,     // Dup Bin → DupBin
    pub dce: u32,           // Lit/Dup Drop removed
    pub stack_cancels: u32, // swap swap / rot -rot annihilated
    pub cmp_negates: u32,   // <cmp> 0= → inverse compare
    // ── lower: registers & stack motion ─────────────────────────────────────
    pub instrs: u32,        // final emitted body instructions (set at commit)
    pub stack_ops: u32,     // stack-motion tokens lowered (dup/swap/rot/pick/…)
    pub stack_ops_free: u32,// …of those, ones that emitted ZERO code (pure reindex)
    pub const_mat: u32,     // constants forced into a register (load_imm64)
    pub reg_copies: u32,    // mov for a dup/over of a register value
    pub gp_allocs: u32,     // pool-register allocations
    pub peak_gp: u32,       // peak live GP pool registers (of 7)
    pub peak_fp: u32,       // peak live FP pool registers (of 7)
    pub spills: u32,        // reserve()→settle forced by register pressure
    // ── lower: memory traffic ───────────────────────────────────────────────
    pub stack_loads: u32,   // ldr pulling an entry cell into the window (ensure)
    pub stack_stores: u32,  // str spilling the window to canonical memory (settle)
    pub mem_fetches: u32,   // @ / c@ (data-memory loads)
    pub mem_stores: u32,    // ! / c! (data-memory stores)
    pub local_loads: u32,   // local read served from the LP frame (ldr)
    pub local_hits: u32,    // local read served from a register (recent store, no ldr)
    pub local_spills: u32,  // deferred local stores flushed to the frame at a barrier
    pub redundant_fetches: u32, // @/c@ of a const address already fetched in the
                            // window — a hot value re-read from memory instead of
                            // kept in a register (the heat-policy gap / promote_hot_cells)
    pub dsp_adjusts: u32,   // add/sub DSP at settle boundaries
    pub calls: u32,         // Call tokens (non-inlined words → settle barriers)
    pub settles: u32,       // settle() invocations
}

impl Metrics {
    /// Fold another run's metrics into this one (peaks max, the rest sum).
    pub fn merge(&mut self, o: &Metrics) {
        self.toks_in += o.toks_in;
        self.toks_out += o.toks_out;
        self.const_folds += o.const_folds;
        self.imm_folds += o.imm_folds;
        self.imm_chains += o.imm_chains;
        self.dup_fuses += o.dup_fuses;
        self.dce += o.dce;
        self.stack_cancels += o.stack_cancels;
        self.cmp_negates += o.cmp_negates;
        self.stack_ops += o.stack_ops;
        self.stack_ops_free += o.stack_ops_free;
        self.const_mat += o.const_mat;
        self.reg_copies += o.reg_copies;
        self.gp_allocs += o.gp_allocs;
        self.peak_gp = self.peak_gp.max(o.peak_gp);
        self.peak_fp = self.peak_fp.max(o.peak_fp);
        self.spills += o.spills;
        self.stack_loads += o.stack_loads;
        self.stack_stores += o.stack_stores;
        self.mem_fetches += o.mem_fetches;
        self.mem_stores += o.mem_stores;
        self.local_loads += o.local_loads;
        self.local_hits += o.local_hits;
        self.local_spills += o.local_spills;
        self.redundant_fetches += o.redundant_fetches;
        self.dsp_adjusts += o.dsp_adjusts;
        self.calls += o.calls;
        self.settles += o.settles;
    }
    /// Total reduce-pass rewrites (a proxy for token-level effectiveness).
    pub fn rewrites(&self) -> u32 {
        self.const_folds + self.imm_folds + self.imm_chains + self.dup_fuses
            + self.dce + self.stack_cancels + self.cmp_negates
    }
}

/// Reduce a token run: const-fold `Lit Lit Bin`, immediate-fold `Lit Bin`,
/// dup-fuse `Dup Bin`, and DCE `Lit Drop` / `Dup Drop`. A single forward pass
/// with lookback on the output is a fixpoint for these linear stack rewrites.
/// Accumulates token-level metrics into `m`.
pub fn reduce(toks: &[Tok], m: &mut Metrics) -> Vec<Tok> {
    m.toks_in += toks.len() as u32;
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
                        m.const_folds += 1;
                        continue;
                    }
                }
                // immediate-fold: ... Lit k, Bin -> ImmBin(op, k)
                if let Some(Tok::Lit(k)) = out.last().copied() {
                    out.pop();
                    // immediate-immediate chaining: a preceding same-op immediate
                    // absorbs this one. `(x op k1) op k2 = x op (k1∘k2)`; for + and
                    // - the magnitudes add (x-k1-k2 = x-(k1+k2)), the rest use op.
                    if let Some(Tok::ImmBin(prev, k1)) = out.last().copied() {
                        if prev == op {
                            let combined = match op {
                                Bin::Sub => k1.wrapping_add(k),
                                _ => op.eval(k1, k),
                            };
                            out.pop();
                            out.push(Tok::ImmBin(op, combined));
                            m.imm_chains += 1;
                            continue;
                        }
                    }
                    out.push(Tok::ImmBin(op, k));
                    m.imm_folds += 1;
                    continue;
                }
                // dup-fuse: ... Dup, Bin -> DupBin(op)
                if let Some(Tok::Stk(Stk::Dup)) = out.last().copied() {
                    out.pop();
                    out.push(Tok::DupBin(op));
                    m.dup_fuses += 1;
                    continue;
                }
                out.push(t);
            }
            Tok::Stk(Stk::Drop) => match out.last().copied() {
                // DCE: a literal or a dup immediately dropped is dead
                Some(Tok::Lit(_)) | Some(Tok::Stk(Stk::Dup)) => {
                    out.pop();
                    m.dce += 1;
                }
                _ => out.push(t),
            },
            // stack-motion cancellation: inverse permutations annihilate
            Tok::Stk(s) => match (out.last().copied(), s) {
                (Some(Tok::Stk(Stk::Swap)), Stk::Swap)
                | (Some(Tok::Stk(Stk::Rot)), Stk::MinusRot)
                | (Some(Tok::Stk(Stk::MinusRot)), Stk::Rot) => {
                    out.pop();
                    m.stack_cancels += 1;
                }
                _ => out.push(t),
            },
            // logical negation of a comparison: `<cmp> 0=` → the inverse compare.
            Tok::Cmp(Cmp::ZEq) => match out.last().copied() {
                Some(Tok::Cmp(c)) if c.negate().is_some() => {
                    let n = c.negate().unwrap();
                    out.pop();
                    out.push(Tok::Cmp(n));
                    m.cmp_negates += 1;
                }
                _ => out.push(t),
            },
            _ => out.push(t),
        }
    }
    m.toks_out += out.len() as u32;
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
    m: Metrics, // accumulated codegen metrics for this run
    fetched: Vec<i64>, // const addresses @-fetched in the current window (heat probe)
    // Local register cache: index → (pool register holding the current value,
    // dirty = unspilled store). A stored local (dirty) or a HOT read local (≥2
    // reads in the run) is kept resident so subsequent reads are a `mov`, not a
    // frame `ldr`; cold single-read locals are loaded on demand (no caching, no
    // pessimization). hot_locals is the read-≥2 set for this run.
    lcache: std::collections::HashMap<u32, (u32, bool)>,
    hot_locals: std::collections::HashSet<u32>,
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
            m: Metrics::default(),
            fetched: Vec::new(),
            lcache: std::collections::HashMap::new(),
            hot_locals: std::collections::HashSet::new(),
        }
    }

    /// Record an `@`/`c@` of address `a`; flag a redundant re-read (a hot value
    /// pulled from memory again within the window instead of staying in a reg).
    fn note_fetch(&mut self, a: Loc) {
        if let Loc::Const(addr) = a {
            if self.fetched.contains(&addr) {
                self.m.redundant_fetches += 1;
            } else {
                self.fetched.push(addr);
            }
        }
    }

    fn nfree(&self) -> usize {
        POOL.iter().filter(|&&r| !self.used[r as usize]).count()
    }

    fn alloc(&mut self) -> u32 {
        let r = *POOL.iter().find(|&&r| !self.used[r as usize]).expect("alloc: no free reg");
        self.used[r as usize] = true;
        self.m.gp_allocs += 1;
        let live = POOL.iter().filter(|&&r| self.used[r as usize]).count() as u32;
        self.m.peak_gp = self.m.peak_gp.max(live);
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
            self.m.spills += 1;
            self.settle_data();
        }
    }

    /// Ensure the window holds at least `n` cells, pulling from entry memory.
    fn ensure(&mut self, n: usize) {
        while self.vs.len() < n {
            let r = self.alloc();
            self.out.push(ldr_off(r, DSP, (self.consumed * 8) as u32));
            self.m.stack_loads += 1;
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
                self.m.const_mat += 1;
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
                self.m.reg_copies += 1;
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
                self.note_fetch(a);
                let r = self.to_reg(a);
                self.out.push(ldr0(r, r));
                self.m.mem_fetches += 1;
                self.vs.push(Loc::Reg(r));
            }
            Mem::CFetch => {
                self.ensure(1);
                let a = self.vs.pop().unwrap();
                self.note_fetch(a);
                let r = self.to_reg(a);
                self.out.push(ldrb0(r, r));
                self.m.mem_fetches += 1;
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
                self.m.mem_stores += 1;
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
                self.m.mem_stores += 1;
                self.freer(rx);
                self.freer(raddr);
            }
        }
    }

    fn local_fetch(&mut self, i: u32) {
        if let Some(&(r, _)) = self.lcache.get(&i) {
            // resident (stored, or already loaded as a hot local) → reuse via copy
            self.reserve(1);
            let c = self.copy_of(Loc::Reg(r)); // mov; keeps the cache reg intact
            self.vs.push(c);
            self.m.local_hits += 1;
        } else if self.hot_locals.contains(&i) {
            // hot read (≥2 in this run): load once into the cache, then copy
            self.reserve(2);
            let rc = self.alloc();
            self.out.push(ldr_off(rc, LP, i * 8));
            self.m.local_loads += 1;
            self.lcache.insert(i, (rc, false)); // clean — frame still matches
            let c = self.copy_of(Loc::Reg(rc));
            self.vs.push(c);
        } else {
            // cold read (once): load straight to the data stack, no caching
            self.reserve(1);
            let r = self.alloc();
            self.out.push(ldr_off(r, LP, i * 8));
            self.m.local_loads += 1;
            self.vs.push(Loc::Reg(r));
        }
    }

    fn local_store(&mut self, i: u32) {
        self.reserve(2);
        self.ensure(1);
        let a = self.vs.pop().unwrap();
        let r = self.to_reg(a); // value now in r; the cache takes ownership (dirty)
        if let Some((old, _)) = self.lcache.insert(i, (r, true)) {
            if old != r {
                self.freer(old); // a superseded cached value
            }
        }
        // store deferred — flushed to the frame at the next barrier (spill_locals)
    }

    /// At a barrier, flush every DIRTY (unspilled-store) local to its LP frame
    /// slot and free all cache registers (the frame becomes authoritative again;
    /// caller-saved pool registers don't survive the upcoming call anyway).
    fn spill_locals(&mut self) {
        if self.lcache.is_empty() {
            return;
        }
        let cached: Vec<(u32, (u32, bool))> = self.lcache.drain().collect();
        for (i, (r, dirty)) in cached {
            if dirty {
                self.out.push(str_off(r, LP, i * 8));
                self.m.local_spills += 1;
            }
            self.used[r as usize] = false;
        }
    }

    /// `OpenLocals` (inline IR): allocate a nested LP sub-frame and pop the input
    /// locals off the data stack into it.
    fn open_locals(&mut self, total: u32, inputs: u32) {
        self.settle(); // canonical data stack for the input pops; spills locals
        if total > 0 {
            self.out.push(sub_imm(LP, LP, total * 8));
            for i in (0..inputs).rev() {
                self.out.push(str_off(TOS, LP, i * 8)); // local[i] = TOS
                self.out.push(ldr_post(TOS, DSP, 8)); // raise NOS into TOS
            }
        }
    }

    /// `CloseLocals` (inline IR): the locals are dead — drop the cache and pop the
    /// sub-frame off LP.
    fn close_locals(&mut self, total: u32) {
        for (_, (r, _)) in self.lcache.drain() {
            self.used[r as usize] = false; // dead at frame close; no spill
        }
        if total > 0 {
            self.out.push(add_imm(LP, LP, total * 8));
        }
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
        let live = FPOOL.iter().filter(|&&r| self.fused[r as usize]).count() as u32;
        self.m.peak_fp = self.m.peak_fp.max(live);
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
        self.m.settles += 1;
        self.settle_data();
        self.fsettle();
    }

    /// Write the data virtual stack back to the canonical form (TOS in x0, the rest
    /// in memory from DSP) and reset.
    fn settle_data(&mut self) {
        self.spill_locals(); // a barrier flushes deferred local stores to the frame
        if self.vs.is_empty() {
            self.ensure(1); // materialize a TOS from entry memory
        }
        let l = self.vs.len();
        let delta = self.consumed - (l as i64 - 1); // DSP change in cells
        if delta > 0 {
            self.out.push(add_imm(DSP, DSP, (delta * 8) as u32));
            self.m.dsp_adjusts += 1;
        } else if delta < 0 {
            self.out.push(sub_imm(DSP, DSP, ((-delta) * 8) as u32));
            self.m.dsp_adjusts += 1;
        }
        // Pass 1: store deep register entries (freeing their regs), reading x0
        // before it is overwritten by the new TOS.
        for i in 0..l - 1 {
            if let Loc::Reg(r) = self.vs[i] {
                let off = ((l - 2 - i) as u32) * 8;
                self.out.push(str_off(r, DSP, off));
                self.m.stack_stores += 1;
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
        self.fetched.clear(); // a settle ends the window; re-reads after it are fresh
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
pub fn lower(toks: &[Tok], out: &mut Vec<u32>, m: &mut Metrics) {
    let mut low = Low::new(out);
    // Hotness pre-scan: a local read ≥2 times in this run is worth keeping in a
    // register (cold single-reads load on demand — no caching, no pessimization).
    let mut counts: std::collections::HashMap<u32, u32> = std::collections::HashMap::new();
    for t in toks {
        if let Tok::LocalFetch(i) = t {
            *counts.entry(*i).or_default() += 1;
        }
    }
    low.hot_locals = counts.into_iter().filter(|&(_, c)| c >= 2).map(|(i, _)| i).collect();
    for &t in toks {
        match t {
            Tok::Lit(n) => low.lit(n),
            Tok::Bin(op) => low.bin(op),
            Tok::ImmBin(op, k) => low.imm_bin(op, k),
            Tok::DupBin(op) => low.dup_bin(op),
            // Stack-motion tokens: count, and flag the ones that emit no code
            // (the "everything is pick" payoff — pure virtual-stack reindexing).
            Tok::Stk(s) => {
                let before = low.out.len();
                low.stk(s);
                low.m.stack_ops += 1;
                if low.out.len() == before {
                    low.m.stack_ops_free += 1;
                }
            }
            Tok::PickN(n) => {
                let before = low.out.len();
                low.pick_n(n);
                low.m.stack_ops += 1;
                if low.out.len() == before {
                    low.m.stack_ops_free += 1;
                }
            }
            Tok::RollN(n) => {
                let before = low.out.len();
                low.roll_n(n);
                low.m.stack_ops += 1;
                if low.out.len() == before {
                    low.m.stack_ops_free += 1;
                }
            }
            Tok::Cmp(c) => low.cmp(c),
            Tok::Sel(s) => low.sel(s),
            Tok::Mem(mm) => low.mem(mm),
            Tok::LocalFetch(i) => low.local_fetch(i),
            Tok::LocalStore(i) => low.local_store(i),
            Tok::OpenLocals { total, inputs } => low.open_locals(total, inputs),
            Tok::CloseLocals { total } => low.close_locals(total),
            Tok::IvarFetch(off) => low.ivar_fetch(off),
            Tok::IvarStore(off) => low.ivar_store(off),
            Tok::SelfPush => low.self_push(),
            Tok::FLit(bits) => low.flit(bits),
            Tok::FBin(op) => low.fbin(op),
            Tok::FUn(op) => low.fun(op),
            Tok::FStk(s) => {
                let before = low.out.len();
                low.fstk(s);
                low.m.stack_ops += 1;
                if low.out.len() == before {
                    low.m.stack_ops_free += 1;
                }
            }
            Tok::Call(xt) => {
                low.m.calls += 1;
                low.settle();
                emit_call(xt, low.out);
            }
        }
    }
    low.settle();
    m.merge(&low.m);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn red(toks: &[Tok]) -> Vec<Tok> {
        reduce(toks, &mut Metrics::default())
    }

    #[test]
    fn const_fold() {
        // 2 3 + -> Lit 5
        let r = red(&[Tok::Lit(2), Tok::Lit(3), Tok::Bin(Bin::Add)]);
        assert_eq!(r.len(), 1);
        assert!(matches!(r[0], Tok::Lit(5)));
        // 2 3 + 4 * -> Lit 20
        let r = red(&[
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
        let r = red(&[Tok::Bin(Bin::Add), Tok::Lit(5), Tok::Bin(Bin::Add)]);
        assert!(matches!(r.last(), Some(Tok::ImmBin(Bin::Add, 5))));
        // dup + -> DupBin(Add)
        let r = red(&[Tok::Stk(Stk::Dup), Tok::Bin(Bin::Add)]);
        assert!(matches!(r[..], [Tok::DupBin(Bin::Add)]));
        // 7 drop -> nothing
        let r = red(&[Tok::Lit(7), Tok::Stk(Stk::Drop)]);
        assert!(r.is_empty());
    }
}

