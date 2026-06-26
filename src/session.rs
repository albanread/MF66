//! `Mf66Session` — Phase 1 boot harness.
//!
//! Allocates the Forth region (data stack / return stack / user area / locals),
//! assembles the AArch64 kernel through the front-end + `MacJit`, seeds the user
//! area, and drives primitives through `forth_main` using the memory wire-format
//! (`push`/`call`/`stack`), mirroring WF66's `Wf64Session`.

use std::alloc::{alloc_zeroed, dealloc, Layout};
use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::{Context, Result};

use crate::opt::Tok;
use wfasm::backend::Loader;
use wfasm::native_macos::MacJit;
use wfasm::Assembler;

// ── Region layout (byte offsets within the allocation) ───────────────────
const REGION_SIZE: usize = 8 * 1024 * 1024; // 8 MB
const DSTACK_TOP: usize = 0x0008_0000; // data stack: 0..0x80000, grows down from here
const RSTACK_TOP: usize = 0x0010_0000; // return stack: 0x80000..0x100000, grows down
const USER_BASE: usize = 0x0010_0000; // user area: 0x100000..0x180000
const LOCALS_TOP: usize = 0x0020_0000; // locals: 0x180000..0x200000, grows down
const DICT_BASE: usize = 0x0020_0000; // dict code/header heap (HERE grows up)
const DICT_TOP: usize = 0x0060_0000; // = DICT_END; index/overlay arena grows DOWN from here
const VAR_BASE: usize = 0x0060_0000; // data bodies (VAR_HERE grows up)
const VAR_TOP: usize = 0x0080_0000; // = VAR_LIMIT

// ── User-area offsets (must match kernel/macros.masm, adopted from WF66) ──
const UVAR_BASE: usize = 0x00; // the `base` numeric-base variable
const USER_LATEST: usize = 0x10;
const USER_HERE: usize = 0x18;
const USER_DICT_END: usize = 0x20;
const USER_LATESTXT: usize = 0x78;
const USER_VAR_HERE: usize = 0x1820;
const USER_VAR_LIMIT: usize = 0x1828;
const USER_HOST_RSP: usize = 0x58;
const USER_DSP_SAVE: usize = 0x60;
const USER_SP0: usize = 0x68;
const USER_RSP_CURRENT: usize = 0x70;
const USER_HANDLER: usize = 0x80;
const USER_LP0: usize = 0x15B0;
const USER_FP0: usize = 0x1210;
const USER_FSP: usize = 0x1218;
const USER_FTOS_SAVE: usize = 0x1228;
const FP_STACK_TOP: usize = 0x1400; // empty float-stack pointer (top of user_FP_STACK)
/// Scratch region inside the user area (`push_pad`/`poke`/`expect_bytes`).
const USER_PAD: u64 = 0x100;

// ── Header field offsets (must match kernel/macros.masm) ──────────────────
const DH_XTPTR: u64 = 16;
const DH_TFA: u64 = 46;
const TFA_TCOL: u8 = 0x82; // colon-definition type tag

const CELL: usize = 8;

/// `forth_main(target_xt, logical_dsp_in, rsp_top, user_base) -> 0`.
type ForthMain = extern "C" fn(u64, u64, u64, u64) -> u64;

pub struct Mf66Session {
    // Keep the loader alive: the JIT'd code lives in its arena.
    _jit: MacJit,
    forth_main: ForthMain,

    region: *mut u8,
    layout: Layout,

    dstack_top: u64,
    rstack_top: u64,
    user_base: u64,
    /// Current logical data-stack pointer (== dstack_top when empty).
    current_dsp: u64,

    /// Executable region for compiled colon-word bodies.
    code: crate::codearena::CodeArena,
    /// Colon-compiler state (None = interpreting).
    pending: Option<ColonDef>,
    /// Set when `bye` is seen, to stop the REPL.
    bye: bool,
    /// Optimizer vocabulary: primitive xt → the token(s) it inlines to.
    vocab: HashMap<u64, Vec<Tok>>,
    /// Word count of the most recently compiled colon body (for optimizer tests).
    last_body_words: usize,
}

/// In-progress colon definition: the word name, the accumulated body words, and
/// the compile-time control-flow stack of pending branch patches / loop targets.
struct ColonDef {
    name: String,
    body: Vec<u32>,
    cf: Vec<Cf>,
    /// Accumulated straight-line token run (flushed → reduced → lowered into
    /// `body` at each control-flow boundary and at `;`).
    toks: Vec<Tok>,
    /// Local names by slot index (LP-relative frame); empty if none declared.
    locals: Vec<String>,
}

/// A pending control-flow mark (compile time).
enum Cf {
    FwdCbz(usize),   // a forward `cbz` to patch (if / while)
    FwdBcond(usize), // a forward `b.<cond>` to patch (fused cmp + if / while)
    FwdB(usize),     // a forward `b` to patch (else)
    Begin(usize),    // a backward target word index (begin)
    Do(usize),       // a DO-loop body top (loop / +loop branch back here)
}

impl Mf66Session {
    /// Boot with only the built-in runtime externs.
    pub fn new() -> Result<Self> {
        Self::with_externs(&[])
    }

    /// Boot, binding `extra` host externs (name → `extern "C"` address) in
    /// addition to the built-ins. Externs must be bound before the kernel is
    /// assembled, so they are supplied up front.
    pub fn with_externs(extra: &[(&str, *const ())]) -> Result<Self> {
        // 1. Expand the kernel macros to AArch64 text.
        let mut asm = Assembler::new();
        asm.register_macro("stk", crate::asm_macros::stk);
        let main = kernel_path();
        let text = asm
            .assemble_file(&main)
            .map_err(|e| anyhow::anyhow!("assemble {}: {e}", main.display()))?;

        // 2. Load into MAP_JIT memory, binding host externs first.
        let mut jit = MacJit::new();
        for (name, addr) in crate::runtime::externs().iter().chain(extra.iter()) {
            jit.define_extern_fn(name, 0, *addr as *mut std::ffi::c_void)
                .with_context(|| format!("bind extern {name}"))?;
        }
        jit.add_asm(&text)?;
        let forth_main: ForthMain = unsafe { jit.lookup_fn("forth_main")? };

        // 3. Allocate + seed the Forth region.
        let layout = Layout::from_size_align(REGION_SIZE, 4096).unwrap();
        let region = unsafe { alloc_zeroed(layout) };
        if region.is_null() {
            anyhow::bail!("region allocation failed");
        }
        let base = region as u64;
        let dstack_top = base + DSTACK_TOP as u64;
        let rstack_top = base + RSTACK_TOP as u64;
        let user_base = base + USER_BASE as u64;

        let code = crate::codearena::CodeArena::with_capacity(4 * 1024 * 1024)?;
        let mut s = Mf66Session {
            _jit: jit,
            forth_main,
            region,
            layout,
            dstack_top,
            rstack_top,
            user_base,
            current_dsp: dstack_top,
            code,
            pending: None,
            bye: false,
            vocab: HashMap::new(),
            last_body_words: 0,
        };

        s.write_user(USER_RSP_CURRENT, rstack_top);
        s.write_user(USER_LP0, base + LOCALS_TOP as u64);
        s.write_user(USER_SP0, dstack_top);
        s.write_user(USER_FTOS_SAVE, 0);
        s.write_user(USER_FP0, base + FP_STACK_TOP as u64);
        s.write_user(USER_FSP, base + FP_STACK_TOP as u64);
        s.write_user(USER_HANDLER, 0); // no active catch handler
        s.write_user(USER_HOST_RSP, 0);
        s.write_user(USER_DSP_SAVE, dstack_top);
        s.write_user(UVAR_BASE, 10); // decimal default
        // Dictionary heaps (empty): code/header heap grows up from DICT_BASE; the
        // overlay arena grows down from DICT_END; data bodies grow up from VAR_BASE.
        s.write_user(USER_LATEST, 0);
        s.write_user(USER_LATESTXT, 0);
        s.write_user(USER_HERE, base + DICT_BASE as u64);
        s.write_user(USER_DICT_END, base + DICT_TOP as u64);
        s.write_user(USER_VAR_HERE, base + VAR_BASE as u64);
        s.write_user(USER_VAR_LIMIT, base + VAR_TOP as u64);
        // Carve the FORTH wordlist + install the search order, then publish every
        // kernel primitive so it is findable by its Forth name.
        s.call("init_dictionary_overlay")?;
        s.bootstrap_dictionary()?;
        s.build_vocab()?;
        Ok(s)
    }

    // ── Stack API (mirrors Wf64Session) ─────────────────────────────────
    /// Push a cell.
    pub fn push(&mut self, v: i64) {
        self.current_dsp -= CELL as u64;
        unsafe { (self.current_dsp as *mut u64).write(v as u64) };
    }

    /// Data stack contents, top-first.
    pub fn stack(&self) -> Vec<i64> {
        let depth = self.depth();
        (0..depth)
            .map(|i| unsafe { (self.current_dsp as *const u64).add(i).read() as i64 })
            .collect()
    }

    pub fn depth(&self) -> usize {
        ((self.dstack_top - self.current_dsp) / CELL as u64) as usize
    }

    /// Pop and return the top data-stack cell (None if empty).
    fn pop_data(&mut self) -> Option<i64> {
        if self.depth() == 0 {
            return None;
        }
        let v = unsafe { (self.current_dsp as *const u64).read() as i64 };
        self.current_dsp += CELL as u64;
        Some(v)
    }

    /// Resolve a primitive's asm symbol to its execution token (code address).
    /// Used by the corpus harness for NYIMP detection.
    pub fn xt_of(&mut self, asm_sym: &str) -> Result<u64> {
        self._jit
            .lookup_addr(asm_sym)
            .with_context(|| format!("xt_of({asm_sym})"))
    }

    /// Base of the user-area scratch (PAD) region.
    pub fn pad_base(&self) -> u64 {
        self.user_base + USER_PAD
    }

    /// Instruction-word count of the most recently compiled colon body. Lets
    /// tests confirm the optimizer shrank the code (const-fold/inline/fuse).
    pub fn last_body_words(&self) -> usize {
        self.last_body_words
    }

    /// Invoke a primitive by its asm symbol through `forth_main`.
    pub fn call(&mut self, asm_sym: &str) -> Result<()> {
        let xt = self.xt_of(asm_sym)?;
        (self.forth_main)(xt, self.current_dsp, self.rstack_top, self.user_base);
        self.current_dsp = self.read_user(USER_DSP_SAVE);
        Ok(())
    }

    /// Write `name` into PAD and push (c-addr, u) for a dictionary primitive.
    fn push_name(&mut self, name: &str) {
        let pad = self.pad_base();
        let bytes = name.as_bytes();
        unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), pad as *mut u8, bytes.len()) };
        self.push(pad as i64);
        self.push(bytes.len() as i64);
    }

    /// Build a (header-only) dictionary entry for `name` via the kernel `(create)`.
    pub fn create_word(&mut self, name: &str) -> Result<()> {
        self.push_name(name);
        self.call("create")
    }

    /// Publish `name` as a leaf word that pushes the constant `value` at runtime
    /// (body = spill TOS, load value, ret). The foundation for `constant`,
    /// `variable`, and the OOP `class NAME` / `new OBJ` defining words.
    pub fn publish_constant(&mut self, name: &str, value: i64) -> Result<u64> {
        let mut body = Vec::new();
        crate::aenc::emit_lit(value, &mut body); // push old TOS, TOS = value
        body.push(crate::aenc::ret());
        let xt = self.code.commit(&body)?;
        self.push_name(name);
        self.call("create")?;
        let header = self.read_user(USER_LATEST);
        self.write_u64(header + DH_XTPTR, xt);
        self.write_u8(header + DH_TFA, TFA_TCOL);
        Ok(xt)
    }

    /// Allot a zeroed cell in data space and return its address.
    fn allot_cell(&mut self) -> u64 {
        let addr = self.read_user(USER_VAR_HERE);
        self.write_u64(addr, 0);
        self.write_user(USER_VAR_HERE, addr + CELL as u64);
        addr
    }

    /// Look up `name` via the kernel `find-name`. Returns the name-token address
    /// (`nt`) if found, else `None`. Clears the data stack afterward.
    pub fn find(&mut self, name: &str) -> Result<Option<u64>> {
        self.push_name(name);
        self.call("find_name")?;
        let s = self.stack(); // top-first: [-1, nt] if found, else [0, u, c-addr]
        let result = if s.first() == Some(&-1) && s.len() >= 2 {
            Some(s[1] as u64)
        } else {
            None
        };
        self.reset();
        Ok(result)
    }

    /// Publish a kernel primitive into the dictionary (bootstrap helper):
    /// resolve `asm_sym` to its code address and `publish_primitive` it under
    /// `forth_name`. `immediate` marks compile-time words (wired into dh_ct later).
    pub fn publish(&mut self, forth_name: &str, asm_sym: &str, immediate: bool) -> Result<()> {
        let xt = self.xt_of(asm_sym)?;
        self.push_name(forth_name);
        self.push(xt as i64);
        self.push(0); // comp-xt (compile helper) — none yet
        self.push(if immediate { 1 } else { 0 });
        self.call("publish_primitive")
    }

    /// Interpret/compile Forth `text`. In interpret state each token is
    /// found+executed or parsed as a number and pushed. `:` begins a colon
    /// definition (compile state); subsequent tokens are *compiled* into a body
    /// (a call per word, a literal per number) until `;`, which finishes the
    /// word. (Bring-up: tokenizing/number parsing + the `:`/`;` handling are
    /// Rust-side; the kernel parse + interpret loop and immediate words come next.)
    pub fn eval(&mut self, text: &str) -> Result<()> {
        let mut tokens = text.split_whitespace();
        while let Some(tok) = tokens.next() {
            // Comments: `\` to end of line, `( … )` inline.
            if tok == "\\" {
                break;
            }
            if tok == "(" {
                for t in tokens.by_ref() {
                    if t.ends_with(')') {
                        break;
                    }
                }
                continue;
            }
            let lk = tok.to_ascii_lowercase(); // Forth is case-insensitive
            // ' / ['] name — push (interpret) or compile (literal) the word's xt.
            if lk == "'" || lk == "[']" {
                let name = tokens
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("`{tok}` needs a name"))?;
                let xt = self
                    .xt_of_forth_name(name)?
                    .ok_or_else(|| anyhow::anyhow!("undefined word: {name}"))?;
                if self.pending.is_some() {
                    self.pending.as_mut().unwrap().toks.push(Tok::Lit(xt as i64));
                } else {
                    self.push(xt as i64);
                }
                continue;
            }
            if self.pending.is_some() {
                if lk == ";" {
                    self.finish_colon()?;
                } else if lk == "{:" {
                    // {: in0 in1 … | uninit … :}  — declare a locals frame
                    let mut names = Vec::new();
                    let mut inputs = 0usize;
                    let mut after_pipe = false;
                    for t in tokens.by_ref() {
                        if t == ":}" {
                            break;
                        }
                        if t == "|" {
                            after_pipe = true;
                            continue;
                        }
                        if !after_pipe {
                            inputs += 1;
                        }
                        names.push(t.to_string());
                    }
                    self.open_locals(names, inputs);
                } else if lk == "to" {
                    let target = tokens
                        .next()
                        .ok_or_else(|| anyhow::anyhow!("`to` needs a name"))?;
                    self.compile_to(target)?;
                } else {
                    self.compile_token(tok)?;
                }
            } else if lk == "bye" {
                self.bye = true;
                break;
            } else if lk == "constant" {
                let name = tokens
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("`constant` needs a name"))?;
                let v = self
                    .pop_data()
                    .ok_or_else(|| anyhow::anyhow!("`constant` needs a value"))?;
                self.publish_constant(name, v)?;
            } else if lk == "variable" {
                let name = tokens
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("`variable` needs a name"))?;
                let addr = self.allot_cell();
                self.publish_constant(name, addr as i64)?;
            } else if lk == ":" {
                let name = tokens
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("`:` needs a name"))?;
                let mut body = Vec::new();
                crate::aenc::emit_nest(&mut body);
                self.pending = Some(ColonDef {
                    name: name.to_string(),
                    body,
                    cf: Vec::new(),
                    toks: Vec::new(),
                    locals: Vec::new(),
                });
            } else {
                self.interpret_token(tok)?;
            }
        }
        Ok(())
    }

    /// Interpret one token: find+execute, else number→push, else error.
    fn interpret_token(&mut self, tok: &str) -> Result<()> {
        let base = self.read_user(UVAR_BASE) as u32;
        self.push_name(tok);
        self.call("find_name")?;
        if self.stack().first() == Some(&-1) {
            self.call("drop_")?;
            self.call("name_to_interpret")?;
            self.call("execute").with_context(|| format!("execute {tok}"))?;
        } else {
            self.call("drop_")?;
            self.call("drop_")?;
            self.call("drop_")?;
            match parse_forth_int(tok, base) {
                Some(n) => self.push(n),
                None => {
                    let f = parse_forth_float(tok)
                        .ok_or_else(|| anyhow::anyhow!("undefined word: {tok}"))?;
                    self.push(f.to_bits() as i64); // push raw bits, then flit → float stack
                    self.call("flit")?;
                }
            }
        }
        Ok(())
    }

    /// Resolve `tok` to an xt without disturbing the data stack net (find_name +
    /// name>interpret leave the xt, which we read and drop).
    fn xt_of_forth_name(&mut self, tok: &str) -> Result<Option<u64>> {
        self.push_name(tok);
        self.call("find_name")?;
        if self.stack().first() == Some(&-1) {
            self.call("drop_")?; // drop -1
            self.call("name_to_interpret")?; // nt -> xt
            let xt = self.stack()[0] as u64;
            self.call("drop_")?; // drop xt
            Ok(Some(xt))
        } else {
            self.call("drop_")?; // 0
            self.call("drop_")?; // u
            self.call("drop_")?; // c-addr
            Ok(None)
        }
    }

    /// Compile one token into the pending colon body: a control-flow directive,
    /// else a call if it's a word, else a literal if it's a number.
    /// `{: … :}` — allocate a locals frame on LP (x21) and pop the input locals
    /// (declaration order; rightmost input = TOS) into it. Uninitialized locals
    /// (after `|`) just reserve a slot.
    fn open_locals(&mut self, names: Vec<String>, inputs: usize) {
        use crate::aenc::{ldr_post, str_off, sub_imm};
        self.flush_toks();
        let count = names.len();
        let def = self.pending.as_mut().unwrap();
        if count > 0 {
            def.body.push(sub_imm(21, 21, (count * 8) as u32)); // allocate frame
            for i in (0..inputs).rev() {
                def.body.push(str_off(0, 21, (i * 8) as u32)); // local[i] = TOS
                def.body.push(ldr_post(0, 19, 8)); // raise NOS into TOS
            }
        }
        def.locals = names;
    }

    /// `to <local>` — compile a store into the named local.
    fn compile_to(&mut self, target: &str) -> Result<()> {
        let def = self.pending.as_ref().unwrap();
        match def.locals.iter().position(|n| n.eq_ignore_ascii_case(target)) {
            Some(i) => {
                self.pending.as_mut().unwrap().toks.push(Tok::LocalStore(i as u32));
                Ok(())
            }
            None => anyhow::bail!("`to {target}` — not a local (VALUEs not supported yet)"),
        }
    }

    fn compile_token(&mut self, tok: &str) -> Result<()> {
        // A bare local name compiles to a fetch from its frame slot.
        if let Some(i) = self
            .pending
            .as_ref()
            .and_then(|d| d.locals.iter().position(|n| n.eq_ignore_ascii_case(tok)))
        {
            self.pending.as_mut().unwrap().toks.push(Tok::LocalFetch(i as u32));
            return Ok(());
        }
        let lk = tok.to_ascii_lowercase();
        // cmp-branch fusion: a comparison immediately before if/until/while folds
        // into one cmp + b.<cond> (no -1/0 flag materialized).
        if matches!(lk.as_str(), "if" | "until" | "while") {
            if let Some(Tok::Cmp(c)) = self.pending.as_ref().and_then(|d| d.toks.last()).copied() {
                self.pending.as_mut().unwrap().toks.pop(); // detach the cmp
                self.flush_toks(); // lower the rest of the run
                return self.compile_control_fused(&lk, c);
            }
        }
        if matches!(
            lk.as_str(),
            "if" | "else" | "then" | "begin" | "until" | "while" | "repeat" | "do" | "loop" | "+loop"
        ) {
            // Lower the accumulated straight-line run before the branch boundary.
            self.flush_toks();
            return self.compile_control(&lk);
        }
        let base = self.read_user(UVAR_BASE) as u32;
        match self.xt_of_forth_name(tok)? {
            Some(xt) => {
                let toks = self.vocab.get(&xt).cloned().unwrap_or_else(|| vec![Tok::Call(xt)]);
                self.pending.as_mut().unwrap().toks.extend(toks);
            }
            None => match parse_forth_int(tok, base) {
                Some(n) => self.pending.as_mut().unwrap().toks.push(Tok::Lit(n)),
                None => {
                    let f = parse_forth_float(tok)
                        .ok_or_else(|| anyhow::anyhow!("undefined word: {tok}"))?;
                    let flit = self.xt_of("flit")?;
                    let def = self.pending.as_mut().unwrap();
                    def.toks.push(Tok::Lit(f.to_bits() as i64)); // bits …
                    def.toks.push(Tok::Call(flit)); // … then flit → float stack
                }
            },
        }
        Ok(())
    }

    /// Reduce + lower the accumulated token run into the body, then clear it.
    fn flush_toks(&mut self) {
        if let Some(def) = self.pending.as_mut() {
            if !def.toks.is_empty() {
                let reduced = crate::opt::reduce(&def.toks);
                crate::opt::lower(&reduced, &mut def.body);
                def.toks.clear();
            }
        }
    }

    /// Build the optimizer vocabulary: primitive asm symbol → inline token(s).
    fn build_vocab(&mut self) -> Result<()> {
        use crate::opt::{Bin::*, Cmp::*, Mem::*, Stk::*};
        let table: &[(&str, &[Tok])] = &[
            ("plus", &[Tok::Bin(Add)]),
            ("minus", &[Tok::Bin(Sub)]),
            ("times", &[Tok::Bin(Mul)]),
            ("and_", &[Tok::Bin(And)]),
            ("or_", &[Tok::Bin(Or)]),
            ("xor_", &[Tok::Bin(Xor)]),
            ("dup_", &[Tok::Stk(Dup)]),
            ("drop_", &[Tok::Stk(Drop)]),
            ("swap_", &[Tok::Stk(Swap)]),
            ("over_", &[Tok::Stk(Over)]),
            ("nip_", &[Tok::Stk(Nip)]),
            ("equal", &[Tok::Cmp(Eq)]),
            ("not_equal", &[Tok::Cmp(Ne)]),
            ("less", &[Tok::Cmp(Lt)]),
            ("greater", &[Tok::Cmp(Gt)]),
            ("less_equal", &[Tok::Cmp(Le)]),
            ("greater_equal", &[Tok::Cmp(Ge)]),
            ("u_less", &[Tok::Cmp(ULt)]),
            ("u_greater", &[Tok::Cmp(UGt)]),
            ("zero_equal", &[Tok::Cmp(ZEq)]),
            ("zero_not_equal", &[Tok::Cmp(ZNe)]),
            ("zero_less", &[Tok::Cmp(ZLt)]),
            ("zero_greater", &[Tok::Cmp(ZGt)]),
            ("fetch", &[Tok::Mem(Fetch)]),
            ("store", &[Tok::Mem(Store)]),
            ("c_fetch", &[Tok::Mem(CFetch)]),
            ("c_store", &[Tok::Mem(CStore)]),
            ("one_plus", &[Tok::Lit(1), Tok::Bin(Add)]),
            ("one_minus", &[Tok::Lit(1), Tok::Bin(Sub)]),
            ("two_plus", &[Tok::Lit(2), Tok::Bin(Add)]),
            ("two_minus", &[Tok::Lit(2), Tok::Bin(Sub)]),
            ("two_times", &[Tok::Lit(2), Tok::Bin(Mul)]),
            ("cell_plus", &[Tok::Lit(8), Tok::Bin(Add)]),
            ("cells", &[Tok::Lit(8), Tok::Bin(Mul)]),
            ("char_plus", &[Tok::Lit(1), Tok::Bin(Add)]),
            ("negate", &[Tok::Lit(-1), Tok::Bin(Mul)]),
            ("invert", &[Tok::Lit(-1), Tok::Bin(Xor)]),
        ];
        let mut vocab = HashMap::new();
        for (sym, toks) in table {
            if let Ok(xt) = self.xt_of(sym) {
                vocab.insert(xt, toks.to_vec());
            }
        }
        self.vocab = vocab;
        Ok(())
    }

    /// Compile-time control-flow directives (immediate). All operate on the
    /// pending definition's body + control-flow stack; branch offsets are
    /// word-relative. `if`/`while` consume the TOS flag (false → branch).
    fn compile_control(&mut self, tok: &str) -> Result<()> {
        use crate::aenc::{b, emit_flag_test_cbz, patch_b, patch_bcond, patch_cbz};
        let def = self.pending.as_mut().expect("control word outside a definition");
        match tok {
            "if" => {
                let i = emit_flag_test_cbz(&mut def.body);
                def.cf.push(Cf::FwdCbz(i));
            }
            "else" => {
                let bidx = def.body.len();
                def.body.push(b(0)); // jump over the else-clause (patched at `then`)
                let here = def.body.len();
                match def.cf.pop() {
                    Some(Cf::FwdCbz(i)) => patch_cbz(&mut def.body, i, here),
                    Some(Cf::FwdBcond(i)) => patch_bcond(&mut def.body, i, here),
                    _ => anyhow::bail!("`else` without `if`"),
                }
                def.cf.push(Cf::FwdB(bidx));
            }
            "then" => {
                let here = def.body.len();
                match def.cf.pop() {
                    Some(Cf::FwdCbz(i)) => patch_cbz(&mut def.body, i, here),
                    Some(Cf::FwdBcond(i)) => patch_bcond(&mut def.body, i, here),
                    Some(Cf::FwdB(i)) => patch_b(&mut def.body, i, here),
                    _ => anyhow::bail!("`then` without `if`/`else`"),
                }
            }
            "begin" => def.cf.push(Cf::Begin(def.body.len())),
            "until" => {
                let i = emit_flag_test_cbz(&mut def.body); // false → loop back
                match def.cf.pop() {
                    Some(Cf::Begin(t)) => patch_cbz(&mut def.body, i, t),
                    _ => anyhow::bail!("`until` without `begin`"),
                }
            }
            "while" => {
                let i = emit_flag_test_cbz(&mut def.body); // false → exit loop
                def.cf.push(Cf::FwdCbz(i));
            }
            "repeat" => {
                let wbranch = def.cf.pop();
                let target = match def.cf.pop() {
                    Some(Cf::Begin(t)) => t,
                    _ => anyhow::bail!("`repeat` without `begin`"),
                };
                let bidx = def.body.len();
                def.body.push(b(0));
                patch_b(&mut def.body, bidx, target); // branch back to begin
                let here = def.body.len();
                match wbranch {
                    Some(Cf::FwdCbz(i)) => patch_cbz(&mut def.body, i, here),
                    Some(Cf::FwdBcond(i)) => patch_bcond(&mut def.body, i, here),
                    _ => anyhow::bail!("`repeat` without `while`"),
                }
            }
            "do" => {
                crate::aenc::emit_do(&mut def.body);
                def.cf.push(Cf::Do(def.body.len())); // loop body top
            }
            "loop" => {
                let top = match def.cf.pop() {
                    Some(Cf::Do(t)) => t,
                    _ => anyhow::bail!("`loop` without `do`"),
                };
                crate::aenc::emit_loop(&mut def.body, top);
            }
            "+loop" => {
                let top = match def.cf.pop() {
                    Some(Cf::Do(t)) => t,
                    _ => anyhow::bail!("`+loop` without `do`"),
                };
                crate::aenc::emit_plus_loop(&mut def.body, top);
            }
            _ => unreachable!(),
        }
        Ok(())
    }

    /// Fused comparison + control test (cmp-branch fusion): emit the comparison
    /// (consuming its operands), then a single `b.<cond>` on the inverse so the
    /// branch is taken when the Forth test is false — no `-1/0` flag in between.
    fn compile_control_fused(&mut self, tok: &str, c: crate::opt::Cmp) -> Result<()> {
        use crate::aenc::{bcond, patch_bcond};
        let ctrue = {
            let def = self.pending.as_mut().unwrap();
            crate::opt::fused_cmp(c, &mut def.body)
        };
        let binv = ctrue ^ 1; // branch when the comparison is FALSE
        let def = self.pending.as_mut().unwrap();
        match tok {
            "if" | "while" => {
                let i = def.body.len();
                def.body.push(bcond(binv, 0));
                def.cf.push(Cf::FwdBcond(i));
            }
            "until" => {
                let i = def.body.len();
                def.body.push(bcond(binv, 0));
                match def.cf.pop() {
                    Some(Cf::Begin(t)) => patch_bcond(&mut def.body, i, t),
                    _ => anyhow::bail!("`until` without `begin`"),
                }
            }
            _ => unreachable!(),
        }
        Ok(())
    }

    /// Finish the pending colon definition: emit unnest+ret, commit the body to
    /// the code arena, create the header, and point it at the body (tfa = colon).
    fn finish_colon(&mut self) -> Result<()> {
        self.flush_toks(); // lower the final straight-line run
        let mut def = self.pending.take().expect("finish_colon with no pending def");
        if !def.cf.is_empty() {
            anyhow::bail!("unbalanced control flow in `{}`", def.name);
        }
        if !def.locals.is_empty() {
            // free the locals frame before returning
            def.body.push(crate::aenc::add_imm(21, 21, (def.locals.len() * 8) as u32));
        }
        crate::aenc::emit_unnest_ret(&mut def.body);
        self.last_body_words = def.body.len();
        let xt = self.code.commit(&def.body)?;
        // Header (in the RW dict heap) via (create), then patch xt + type tag.
        self.push_name(&def.name);
        self.call("create")?;
        let header = self.read_user(USER_LATEST);
        self.write_u64(header + DH_XTPTR, xt);
        self.write_u8(header + DH_TFA, TFA_TCOL);
        Ok(())
    }

    /// Publish every kernel primitive (the `PRIMITIVES` table) into the dictionary.

    /// Interpret `text` and return everything it printed (via `.`/`emit`/`type`/…).
    pub fn eval_out(&mut self, text: &str) -> Result<String> {
        crate::runtime::capture_clear();
        self.eval(text)?;
        Ok(crate::runtime::capture_take())
    }

    /// Run `input` as a REPL transcript: execute each line, emit ` ok\n` after a
    /// successful line, stop at `bye`. Returns all captured output — matching
    /// WF66's `quit` framing for the eval corpus.
    pub fn repl(&mut self, input: &str) -> Result<String> {
        crate::runtime::capture_clear();
        self.bye = false;
        for line in input.lines() {
            self.eval(line)?;
            if self.bye {
                break;
            }
            crate::runtime::capture_str(" ok\n");
        }
        Ok(crate::runtime::capture_take())
    }

    /// Publish every kernel primitive (the `PRIMITIVES` table) into the dictionary.
    fn bootstrap_dictionary(&mut self) -> Result<()> {
        for &(name, sym, immediate) in crate::primitives::PRIMITIVES {
            self.publish(name, sym, immediate)
                .with_context(|| format!("publish {name} ({sym})"))?;
        }
        Ok(())
    }

    /// Interpret-mode core: find `name` and execute it (`find → name>interpret →
    /// execute`). Any data-stack args pushed beforehand are passed to the word.
    pub fn run_word(&mut self, name: &str) -> Result<()> {
        self.push_name(name);
        self.call("find_name")?;
        if self.stack().first() != Some(&-1) {
            anyhow::bail!("word not found: {name}");
        }
        self.call("drop_")?; // drop the found flag (-1), leaving ( … nt )
        self.call("name_to_interpret")?; // nt -> xt
        self.call("execute") // run it
    }

    /// Clear the data stack and restore post-boot defaults.
    pub fn reset(&mut self) {
        self.current_dsp = self.dstack_top;
        self.write_user(USER_DSP_SAVE, self.dstack_top);
        self.write_user(USER_RSP_CURRENT, self.rstack_top);
        self.write_user(UVAR_BASE, 10);
    }

    // ── helpers ──────────────────────────────────────────────────────────
    fn read_user(&self, off: usize) -> u64 {
        unsafe { ((self.user_base + off as u64) as *const u64).read() }
    }
    fn write_user(&mut self, off: usize, v: u64) {
        unsafe { ((self.user_base + off as u64) as *mut u64).write(v) };
    }
    /// Write a cell at an absolute address in the (RW) dict heap.
    fn write_u64(&mut self, addr: u64, v: u64) {
        unsafe { (addr as *mut u64).write_unaligned(v) };
    }
    /// Write a byte at an absolute address in the (RW) dict heap.
    fn write_u8(&mut self, addr: u64, v: u8) {
        unsafe { (addr as *mut u8).write(v) };
    }
}

impl Drop for Mf66Session {
    fn drop(&mut self) {
        unsafe { dealloc(self.region, self.layout) };
    }
}

/// Locate `kernel/main.masm` relative to the crate root.
fn kernel_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("kernel/main.masm")
}

/// Parse a Forth floating-point literal (`1.5`, `1.5e0`, `-2.25`, `1e3`). Only a
/// token that has a digit and a `.` or exponent is considered, and `inf`/`nan`
/// are rejected, so integers and word names never match.
fn parse_forth_float(tok: &str) -> Option<f64> {
    let has_digit = tok.bytes().any(|b| b.is_ascii_digit());
    let floaty = tok.contains('.') || tok.contains('e') || tok.contains('E');
    if !has_digit || !floaty {
        return None;
    }
    match tok.parse::<f64>() {
        Ok(f) if f.is_finite() => Some(f),
        _ => None,
    }
}

/// Parse a Forth integer literal in `base` (2..=36), allowing a leading `-` and
/// explicit radix prefixes `0x`/`$` (hex), `0b`/`%` (binary), `0o` (octal).
fn parse_forth_int(tok: &str, base: u32) -> Option<i64> {
    let (neg, rest) = match tok.strip_prefix('-') {
        Some(r) => (true, r),
        None => (false, tok),
    };
    let (radix, digits) = if let Some(d) = rest.strip_prefix("0x").or_else(|| rest.strip_prefix("0X")) {
        (16, d)
    } else if let Some(d) = rest.strip_prefix('$') {
        (16, d)
    } else if let Some(d) = rest.strip_prefix("0b").or_else(|| rest.strip_prefix("0B")) {
        (2, d)
    } else if let Some(d) = rest.strip_prefix('%') {
        (2, d)
    } else if let Some(d) = rest.strip_prefix("0o") {
        (8, d)
    } else {
        (base.clamp(2, 36), rest)
    };
    if digits.is_empty() {
        return None;
    }
    let mag = i64::from_str_radix(digits, radix).ok()?;
    Some(if neg { mag.wrapping_neg() } else { mag })
}
