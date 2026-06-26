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

const TOS: u32 = 0;
const DSP: u32 = 19;
const NOS: u32 = 9; // scratch for the popped second-on-stack

/// Lower one reduced token run to AArch64 (appended to `out`).
pub fn lower(toks: &[Tok], out: &mut Vec<u32>) {
    for &t in toks {
        match t {
            Tok::Lit(n) => emit_lit(n, out),
            Tok::Bin(op) => {
                out.push(ldr_post(NOS, DSP, 8)); // pop NOS into x9
                bin_nos_tos(op, out);
            }
            Tok::ImmBin(op, k) => imm_bin(op, k, out),
            Tok::DupBin(op) => dup_bin(op, out),
            Tok::Stk(s) => stk(s, out),
            Tok::Cmp(c) => cmp(c, out),
            Tok::Mem(m) => mem(m, out),
            Tok::Call(xt) => emit_call(xt, out),
        }
    }
}

/// `x0 = x9 <op> x0` for Sub (NOS - TOS), else `x0 = x0 <op> x9` (commutative).
fn bin_nos_tos(op: Bin, out: &mut Vec<u32>) {
    out.push(match op {
        Bin::Add => add_reg(TOS, TOS, NOS),
        Bin::Sub => sub_reg(TOS, NOS, TOS), // NOS - TOS
        Bin::Mul => mul(TOS, TOS, NOS),
        Bin::And => and_reg(TOS, TOS, NOS),
        Bin::Or => orr_reg(TOS, TOS, NOS),
        Bin::Xor => eor_reg(TOS, TOS, NOS),
    });
}

/// `TOS = TOS <op> k`.
fn imm_bin(op: Bin, k: i64, out: &mut Vec<u32>) {
    let in_imm12 = (0..=4095).contains(&k);
    let neg_imm12 = (-4095..0).contains(&k);
    match op {
        Bin::Add if in_imm12 => out.push(add_imm(TOS, TOS, k as u32)),
        Bin::Add if neg_imm12 => out.push(sub_imm(TOS, TOS, (-k) as u32)),
        Bin::Sub if in_imm12 => out.push(sub_imm(TOS, TOS, k as u32)),
        Bin::Sub if neg_imm12 => out.push(add_imm(TOS, TOS, (-k) as u32)),
        _ => {
            load_imm64(NOS, k as u64, out); // k -> x9
            out.push(match op {
                Bin::Add => add_reg(TOS, TOS, NOS),
                Bin::Sub => sub_reg(TOS, TOS, NOS), // TOS - k
                Bin::Mul => mul(TOS, TOS, NOS),
                Bin::And => and_reg(TOS, TOS, NOS),
                Bin::Or => orr_reg(TOS, TOS, NOS),
                Bin::Xor => eor_reg(TOS, TOS, NOS),
            });
        }
    }
}

/// `dup` then `op`: the value combined with itself.
fn dup_bin(op: Bin, out: &mut Vec<u32>) {
    match op {
        Bin::Add => out.push(add_reg(TOS, TOS, TOS)), // 2*a
        Bin::Mul => out.push(mul(TOS, TOS, TOS)),     // a*a
        Bin::And | Bin::Or => {}                       // a&a = a|a = a (no-op)
        Bin::Sub | Bin::Xor => out.push(movz(TOS, 0, 0)), // a-a = a^a = 0
    }
}

fn stk(s: Stk, out: &mut Vec<u32>) {
    match s {
        Stk::Dup => out.push(str_pre(TOS, DSP, -8)),
        Stk::Drop => out.push(ldr_post(TOS, DSP, 8)),
        Stk::Swap => {
            out.push(ldr0(NOS, DSP)); // x9 = NOS
            out.push(str_off(TOS, DSP, 0)); // NOS = TOS
            out.push(mov_reg(TOS, NOS)); // TOS = old NOS
        }
        Stk::Over => {
            out.push(ldr0(NOS, DSP)); // x9 = NOS (a)
            out.push(str_pre(TOS, DSP, -8)); // push TOS (b)
            out.push(mov_reg(TOS, NOS)); // TOS = a
        }
        Stk::Nip => out.push(add_imm(DSP, DSP, 8)), // drop NOS
    }
}

fn cmp(c: Cmp, out: &mut Vec<u32>) {
    match c {
        // zero-compares: only TOS
        Cmp::ZEq => {
            out.push(cmp_imm(TOS, 0));
            out.push(csetm(TOS, EQ));
        }
        Cmp::ZNe => {
            out.push(cmp_imm(TOS, 0));
            out.push(csetm(TOS, NE));
        }
        Cmp::ZLt => {
            out.push(cmp_imm(TOS, 0));
            out.push(csetm(TOS, LT));
        }
        Cmp::ZGt => {
            out.push(cmp_imm(TOS, 0));
            out.push(csetm(TOS, GT));
        }
        // binary compares: NOS vs TOS
        _ => {
            out.push(ldr_post(NOS, DSP, 8)); // x9 = NOS
            let (rn, rm, cond) = match c {
                Cmp::Eq => (TOS, NOS, EQ),  // symmetric
                Cmp::Ne => (TOS, NOS, NE),
                Cmp::Lt => (NOS, TOS, LT),  // NOS < TOS
                Cmp::Gt => (NOS, TOS, GT),
                Cmp::Le => (NOS, TOS, LE),
                Cmp::Ge => (NOS, TOS, GE),
                Cmp::ULt => (NOS, TOS, LO),
                Cmp::UGt => (NOS, TOS, HI),
                _ => unreachable!(),
            };
            out.push(cmp_reg(rn, rm));
            out.push(csetm(TOS, cond));
        }
    }
}

fn mem(m: Mem, out: &mut Vec<u32>) {
    match m {
        Mem::Fetch => out.push(ldr0(TOS, TOS)), // x0 = [x0]
        Mem::CFetch => out.push(ldrb0(TOS, TOS)),
        Mem::Store => {
            out.push(ldr_post(NOS, DSP, 8)); // x9 = value (NOS)
            out.push(str_off(NOS, TOS, 0)); // [addr] = value
            out.push(ldr_post(TOS, DSP, 8)); // raise new TOS (drop addr)
        }
        Mem::CStore => {
            out.push(ldr_post(NOS, DSP, 8));
            out.push(strb0(NOS, TOS));
            out.push(ldr_post(TOS, DSP, 8));
        }
    }
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
