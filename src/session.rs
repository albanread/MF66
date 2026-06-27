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
const USER_CURRENT: usize = 0x1500; // wid receiving new defs (bucket array base)
const USER_INDEX_HERE: usize = 0x1518; // overlay node arena (grows down)
const USER_INDEX_LATEST: usize = 0x1520; // newest overlay node (creation chain)
const USER_VAR_HERE: usize = 0x1820;
const USER_VAR_LIMIT: usize = 0x1828;
// Overlay (dn_) node field offsets + bucket mask, for marker/forget rollback.
const DN_BUCKET_NEXT: u64 = 0;
const DN_GLOBAL_PREV: u64 = 8;
const DN_WID: u64 = 24;
const DN_HASH: u64 = 32; // u32 in the low 4 bytes
const DN_HEADER: u64 = 16;
const DN_SIZE: u64 = 40;
const WL_BUCKET_MASK: u64 = 511;
const DH_LINK: u64 = 0; // prev header in the LATEST chain
const DH_NT: u64 = 47; // name-length byte; the name token nt = header + dh_nt
const USER_HOST_RSP: usize = 0x58;
const USER_DSP_SAVE: usize = 0x60;
const USER_SP0: usize = 0x68;
const USER_RSP_CURRENT: usize = 0x70;
const USER_HANDLER: usize = 0x80;
const USER_SELF: usize = 0x1830;
const USER_HOLD: usize = 0x1838;
const USER_HOLD_END: usize = 0x1840;
const HOLD_END_OFF: usize = 0x1900; // hold buffer grows down from here (0x1880..0x1900)
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

    // ── OOP state ──────────────────────────────────────────────────────────
    /// Classes by name (metadata; the source of truth alongside the data-space
    /// class struct each one allocates).
    classes: HashMap<String, ClassInfo>,
    /// Selector names → stable id (index); never cleared (WF66 semantics).
    selectors: Vec<String>,
    /// Named instances → (object base addr, class name).
    objects: HashMap<String, (u64, String)>,
    /// `value` names → the data cell holding the value (for `to`).
    values: HashMap<String, u64>,
    /// Words that push a fixed address (variables, buffers, `create`d words with
    /// no `does>`). In compiled code their name folds to a literal address — the
    /// address never changes — so `v @`/`v !` lower to a register ldr/str instead
    /// of a veneer call.
    var_addrs: HashMap<String, u64>,
    /// Every CREATE-instance's data-field address (incl. does>-having ones like
    /// `2value`/`defer`), so `is`/`2to`/`+to` can write into it.
    instance_addrs: HashMap<String, u64>,
    /// `marker` name → the snapshot to roll the dictionary back to.
    markers: HashMap<String, Marker>,
    /// Per-word optimizer metrics, keyed by definition name (for the report).
    word_metrics: HashMap<String, crate::opt::Metrics>,
    /// CREATE/DOES> defining words → (build tokens incl. `create`, does-code xt
    /// or 0). Replayed in Rust at use time. See finish_defining/instantiate.
    defining_words: HashMap<String, (Vec<String>, u64)>,
    /// Class being built between `class`/`subclass` and `end-class`.
    pending_class: Option<ClassInfo>,
    /// While compiling a `:m` method body: the owning class name (ivar scope).
    method_class: Option<String>,
    /// One-shot flag: the next `->` is a `super` send (early-bind to the parent).
    super_pending: bool,
    /// `[` … `]` — interpret tokens while a definition is open (compile-time eval).
    bracket_interpret: bool,
    /// Static class of the most recently emitted receiver (for early binding).
    last_receiver_class: Option<String>,
    /// Cached xts: (dnu) default method, (send), (send-xt).
    dnu_xt: u64,
    send_xt: u64,
    send_xt_xt: u64,
}

/// OOP class metadata. The data-space class struct mirrors this: [+0]=super
/// struct addr, [+8]=reserved, [+16]=vtable[VTABLE_CAP] (selector id → method xt,
/// unused slots = (dnu)).
#[derive(Clone)]
struct ClassInfo {
    name: String,
    super_name: Option<String>,
    struct_addr: u64,
    isize_bytes: u64,              // instance size incl. the leading class-ptr cell
    ivars: Vec<(String, u64)>,    // ivar name → byte offset within an instance
    methods: HashMap<String, u64>, // selector name → compiled method xt
}

/// Fixed vtable capacity per class (matches the hand-built oop_send layout).
const VTABLE_CAP: u64 = 256;

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
    /// `:noname` — finish pushes the xt instead of creating a header.
    noname: bool,
    /// The code address this body will commit to (for `recurse`).
    self_xt: u64,
    /// `: name create … [does> …] ;` is a defining word — its body is captured
    /// raw (here) and replayed at use time rather than compiled.
    defining: bool,
    raw: Vec<String>,
    /// Forward-branch indices from each `exit`, patched to the single epilogue.
    exits: Vec<usize>,
    /// Optimizer metrics accumulated across this definition's runs.
    metrics: crate::opt::Metrics,
}

/// A `marker` snapshot: the dictionary/data/code state to restore when the
/// marker word is executed (forgetting everything defined after it).
struct Marker {
    latest: u64,
    here: u64,
    var_here: u64,
    index_here: u64,
    index_latest: u64,
    code_cursor: usize,
    classes: HashMap<String, ClassInfo>,
    objects: HashMap<String, (u64, String)>,
    values: HashMap<String, u64>,
    var_addrs: HashMap<String, u64>,
    instance_addrs: HashMap<String, u64>,
    defining_words: HashMap<String, (Vec<String>, u64)>,
}

/// A pending control-flow mark (compile time).
enum Cf {
    FwdCbz(usize),   // a forward `cbz` to patch (if / while)
    FwdBcond(usize), // a forward `b.<cond>` to patch (fused cmp + if / while)
    FwdB(usize),     // a forward `b` to patch (else)
    Begin(usize),    // a backward target word index (begin)
    Do(usize, Vec<usize>), // DO-loop body top + forward branches (?do skip, leave)
    Case,            // a CASE marker (endcase closes every of/endof `then` to here)
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
            classes: HashMap::new(),
            selectors: Vec::new(),
            objects: HashMap::new(),
            pending_class: None,
            method_class: None,
            super_pending: false,
            bracket_interpret: false,
            values: HashMap::new(),
            defining_words: HashMap::new(),
            var_addrs: HashMap::new(),
            instance_addrs: HashMap::new(),
            markers: HashMap::new(),
            word_metrics: HashMap::new(),
            last_receiver_class: None,
            dnu_xt: 0,
            send_xt: 0,
            send_xt_xt: 0,
        };

        s.write_user(USER_RSP_CURRENT, rstack_top);
        s.write_user(USER_LP0, base + LOCALS_TOP as u64);
        s.write_user(USER_SP0, dstack_top);
        s.write_user(USER_FTOS_SAVE, 0);
        s.write_user(USER_FP0, base + FP_STACK_TOP as u64);
        s.write_user(USER_FSP, base + FP_STACK_TOP as u64);
        s.write_user(USER_HANDLER, 0); // no active catch handler
        s.write_user(USER_HOLD_END, base + HOLD_END_OFF as u64);
        s.write_user(USER_HOLD, base + HOLD_END_OFF as u64);
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
        s.oop_boot()?;
        s.bootstrap_lib()?;
        Ok(s)
    }

    /// Standard library words defined in Forth at boot (the lib/core.f analogue).
    fn bootstrap_lib(&mut self) -> Result<()> {
        // Pictured numeric output (double-cell, ANS): # converts one digit of the
        // unsigned double ud; #s converts all remaining (at least one).
        self.eval(": # base@ ud/mod rot dup 9 > if 7 + then 48 + hold ;")?;
        self.eval(": #s begin # 2dup or 0= until ;")?;
        // FP address arithmetic (a float is one cell on this target).
        self.eval(": float+ cell+ ;")?;
        self.eval(": floats cells ;")?;
        self.eval(": faligned aligned ;")?;
        self.eval(": falign ;")?; // data space is already cell-aligned
        // Misc Core-ext composed from existing primitives.
        self.eval(": ?: rot if drop else nip then ;")?; // ( f a b -- a|b )
        self.eval(": true -1 ;")?;
        self.eval(": false 0 ;")?;
        self.eval(": ?negate 0< if negate then ;")?; // ( n1 n2 -- n1|-n1 )
        self.eval(": under+ rot + swap ;")?; // ( a b c -- a+c b )
        self.eval(": d>s drop ;")?; // ( d -- n )
        // FP comparisons + helpers (compose from kernel f< / f- / f0= / f** / …)
        self.eval(": f= f- f0= ;")?;
        self.eval(": f> fswap f< ;")?;
        self.eval(": f<= fswap f< 0= ;")?;
        self.eval(": f>= f< 0= ;")?;
        self.eval(": f2* 2e f* ;")?;
        self.eval(": f2/ 2e f/ ;")?;
        self.eval(": fmax fover fover f< if fswap then fdrop ;")?;
        self.eval(": fmin fover fover f< 0= if fswap then fdrop ;")?;
        self.eval(": falog 10e fswap f** ;")?;
        // Double-cell comparisons (compose from kernel d< / d> / d0= / d0<)
        self.eval(": d<= d> 0= ;")?;
        self.eval(": d>= d< 0= ;")?;
        self.eval(": d0<> d0= 0= ;")?;
        self.eval(": d0>= d0< 0= ;")?;
        self.eval(": d0<= 2dup d0= -rot d0< or ;")?;
        self.eval(": d0> d0<= 0= ;")?;
        self.eval(": dmax 2over 2over d< if 2swap then 2drop ;")?;
        self.eval(": dmin 2over 2over d< 0= if 2swap then 2drop ;")?;
        self.eval(": du<= du> 0= ;")?;
        self.eval(": du>= du< 0= ;")?;
        self.eval(": m+ s>d d+ ;")?; // ( d n -- d' )
        // Float helpers composed from the FP primitives.
        self.eval(": f<> f= 0= ;")?;
        self.eval(": f0<> f0= 0= ;")?;
        // Misc convenience.
        self.eval(": third 2 pick ;")?; // ( a b c -- a b c a )
        self.eval(": environment? 2drop 0 ;")?; // ( c-addr u -- false ) unknown query
        // String helpers (compose from /string / compare / search)
        self.eval(": -trailing begin dup if 2dup + 1- c@ 32 = else 0 then while 1- repeat ;")?;
        self.eval(": -leading begin dup if over c@ 32 = else 0 then while 1 /string repeat ;")?;
        self.eval(": starts-with? rot over < if 2drop drop 0 else tuck compare 0= then ;")?;
        self.eval(": contains? search nip nip ;")?;
        self.eval(": blank 32 fill ;")?; // ( c-addr u -- ) fill with spaces
        // Defining words, now built the real Forth way on CREATE/DOES>.
        self.eval(": constant create , does> @ ;")?;
        self.eval(": variable create 0 , ;")?; // pushes the cell address
        self.eval(": 2variable create 0 , 0 , ;")?;
        // Structures (CREATE/DOES> all the way down).
        self.eval(": field: aligned create dup , cell + does> @ + ;")?;
        self.eval(": cfield: create dup , 1 + does> @ + ;")?;
        self.eval(": 2field: aligned create dup , 16 + does> @ + ;")?;
        self.eval(": begin-structure create here 0 , 0 does> @ ;")?;
        self.eval(": end-structure swap ! ;")?;
        // File access modes (low 3 bits select read/write; bin is a no-op here).
        self.eval("1 constant r/o")?;
        self.eval("2 constant w/o")?;
        self.eval("3 constant r/w")?;
        self.eval(": bin ;")?;
        // Locals-frame introspection: lp@ / lp0@ are primitives; lp-limit is the
        // region floor (MF66's locals area is 512 KiB). lp-smoke exercises a frame.
        self.eval(": lp-limit lp0@ 524288 - ;")?;
        self.eval(": lp-smoke {: | a :} 42 ;")?;
        // ends-with? — needs `variable`, so it's defined here after the defining
        // words are bootstrapped (ported from WF66 core.f).
        self.eval("variable ew-suffix-u variable ew-suffix-addr")?;
        self.eval(
            ": ends-with? ew-suffix-u ! ew-suffix-addr ! \
               dup ew-suffix-u @ < if 2drop 0 \
               else ew-suffix-u @ - + ew-suffix-u @ ew-suffix-addr @ ew-suffix-u @ compare 0= then ;",
        )?;
        // REPLACES / SUBSTITUTE (Forth-2012 String-ext), ported from WF66 core.f
        // (wordlist hiding dropped — all words land in the main dictionary).
        self.eval("16 constant subst-max")?;
        self.eval("create subst-table subst-max 4 cells * allot")?;
        self.eval("variable subst-count")?;
        self.eval("create subst-heap 2048 allot")?;
        self.eval("variable subst-here")?;
        self.eval(": subst-init subst-heap subst-here ! 0 subst-count ! ;")?;
        self.eval("subst-init")?;
        self.eval(": subst-slot 4 cells * subst-table + ;")?;
        self.eval(": subst-name dup @ swap cell+ @ ;")?;
        self.eval(": subst-val dup 2 cells + @ swap 3 cells + @ ;")?;
        self.eval(": subst-alloc >r subst-here @ 2dup r@ cmove nip r@ subst-here +! r> ;")?;
        self.eval(
            ": subst-find subst-count @ 0 ?do \
               2dup i subst-slot subst-name compare 0= if 2drop i true unloop exit then \
               loop 2drop false ;",
        )?;
        self.eval(
            ": replaces 2dup subst-find if \
               >r 2drop subst-alloc r> subst-slot >r r@ 3 cells + ! r> 2 cells + ! \
               else subst-count @ subst-max < if \
               subst-count @ subst-slot >r 1 subst-count +! \
               subst-alloc r@ cell+ ! r@ ! subst-alloc r@ 3 cells + ! r> 2 cells + ! \
               else 2drop 2drop then then ;",
        )?;
        self.eval(
            "variable sub-src variable sub-srclen variable sub-dst \
             variable sub-dstmax variable sub-dstlen variable sub-count",
        )?;
        self.eval(
            ": sub-emit sub-dstlen @ sub-dstmax @ < if \
               sub-dst @ sub-dstlen @ + c! 1 sub-dstlen +! else drop then ;",
        )?;
        self.eval(": sub-emit-str bounds ?do i c@ sub-emit loop ;")?;
        self.eval(": sub-advance 1 sub-src +! -1 sub-srclen +! ;")?;
        self.eval(": sub-peek sub-src @ c@ ;")?;
        self.eval(
            ": substitute sub-dstmax ! sub-dst ! sub-srclen ! sub-src ! \
               0 sub-dstlen ! 0 sub-count ! \
               begin sub-srclen @ while \
               sub-peek [char] % <> if sub-peek sub-emit sub-advance \
               else sub-advance sub-srclen @ 0= if [char] % sub-emit \
               else sub-peek [char] % = if [char] % sub-emit sub-advance \
               else sub-src @ \
               begin sub-srclen @ if sub-peek [char] % <> else 0 then while sub-advance repeat \
               sub-src @ over - sub-srclen @ if sub-advance then \
               2dup subst-find if >r 2drop r> subst-slot subst-val sub-emit-str 1 sub-count +! \
               else [char] % sub-emit sub-emit-str [char] % sub-emit then \
               then then then \
               repeat sub-dst @ sub-dstlen @ sub-count @ ;",
        )?;
        // Limit constants, spacing, base-printers and misc utilities (WF66 core.f).
        self.eval("32 constant bl")?;
        self.eval(": space bl emit ;")?;
        self.eval(": spaces 0 max 0 ?do space loop ;")?;
        self.eval("-1 constant max-u")?;
        self.eval("-1 1 rshift constant max-n")?; // 2^63 - 1
        self.eval("1 63 lshift constant min-n")?; // -2^63
        self.eval("255 constant max-char")?;
        self.eval(": noop ;")?;
        self.eval(": char- 1- ;")?; // ( c-addr -- c-addr-1 )
        self.eval(": hex. base @ >r hex . r> base ! ;")?;
        self.eval(": bin. base @ >r 2 base ! . r> base ! ;")?;
        self.eval(": oct. base @ >r 8 base ! . r> base ! ;")?;
        self.eval(": dec. base @ >r decimal . r> base ! ;")?;
        self.eval(": assert rot 0= if type cr abort else 2drop then ;")?; // ( flag c-addr u -- )
        self.eval(": ud.r >r <# #s #> r> over - 0 max spaces type ;")?; // ( ud width -- )
        // 2VALUE / DEFER via CREATE/DOES> (2to / is write into their cells).
        self.eval(": 2value create , , does> 2@ ;")?;
        self.eval(": defer create 0 , does> @ ?dup if execute then ;")?;
        // Facility terminal stubs (no cursor addressing in this host).
        self.eval(": at-xy 2drop ;")?;
        self.eval(": page ;")?;
        // unused = data-space remaining; >float wraps the host float parser.
        let var_limit = self.read_user(USER_VAR_LIMIT);
        self.publish_constant("(var-limit)", var_limit as i64)?;
        self.eval(": unused (var-limit) here - ;")?;
        self.eval(": >float (parse-float) if flit true else drop false then ;")?;
        Ok(())
    }

    /// One-time OOP setup: the user_SELF slot, the `cell` constant, the root
    /// class `object`, and cached xts for dispatch + (dnu).
    fn oop_boot(&mut self) -> Result<()> {
        self.write_user(USER_SELF, 0);
        self.publish_constant("cell", CELL as i64)?; // `cell ivar: n`
        let pad = self.user_base + USER_PAD;
        self.publish_constant("pad", pad as i64)?; // scratch buffer address
        self.dnu_xt = self.xt_of("dnu_word")?;
        self.send_xt = self.xt_of("send_word")?;
        self.send_xt_xt = self.xt_of("send_xt_word")?;
        // Root class `object`: metadata-only (never instantiated directly).
        self.classes.insert(
            "object".to_string(),
            ClassInfo {
                name: "object".to_string(),
                super_name: None,
                struct_addr: 0,
                isize_bytes: CELL as u64, // just the class-ptr cell
                ivars: Vec::new(),
                methods: HashMap::new(),
            },
        );
        Ok(())
    }

    /// Reverse-lookup a class name by its data-space struct address.
    fn class_name_by_struct(&self, addr: u64) -> Option<String> {
        self.classes
            .iter()
            .find(|(_, c)| c.struct_addr == addr && addr != 0)
            .map(|(n, _)| n.clone())
    }

    /// If compiling a method body and `name` is an ivar of its class, its offset.
    fn ivar_offset(&self, name: &str) -> Option<u32> {
        self.method_class.as_ref()?;
        let c = self.pending_class.as_ref()?;
        c.ivars.iter().find(|(n, _)| n == name).map(|(_, o)| *o as u32)
    }

    /// Find-or-assign a stable selector id for `name`.
    fn selector_id(&mut self, name: &str) -> u64 {
        if let Some(i) = self.selectors.iter().position(|s| s == name) {
            return i as u64;
        }
        self.selectors.push(name.to_string());
        (self.selectors.len() - 1) as u64
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

    /// Publish `name` as a `value` — a word that fetches its data cell at runtime
    /// (body = spill TOS, load cell addr, ldr). `to name` rewrites the cell.
    pub fn publish_value(&mut self, name: &str, cell: u64) -> Result<u64> {
        let mut body = Vec::new();
        body.push(crate::aenc::str_pre(0, 19, -8)); // push: spill old TOS
        crate::aenc::load_imm64(0, cell, &mut body); // TOS = cell addr
        body.push(crate::aenc::ldr0(0, 0)); // TOS = [cell]
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
    /// Store raw bytes at HERE and bump HERE (cell-aligned). Returns (addr, len).
    fn store_string(&mut self, bytes: &[u8]) -> (u64, u64) {
        let addr = self.read_user(USER_VAR_HERE);
        for (i, c) in bytes.iter().enumerate() {
            self.write_u8(addr + i as u64, *c);
        }
        let len = bytes.len() as u64;
        self.write_user(USER_VAR_HERE, (addr + len + 7) & !7);
        (addr, len)
    }

    /// Store a counted string (length byte + bytes) at HERE. Returns its address.
    fn store_counted(&mut self, bytes: &[u8]) -> u64 {
        let addr = self.read_user(USER_VAR_HERE);
        self.write_u8(addr, bytes.len() as u8);
        for (i, c) in bytes.iter().enumerate() {
            self.write_u8(addr + 1 + i as u64, *c);
        }
        let total = bytes.len() as u64 + 1;
        self.write_user(USER_VAR_HERE, (addr + total + 7) & !7);
        addr
    }

    /// Push (interpret) or compile (Lit) a single value.
    fn push_or_compile_lit(&mut self, v: i64) {
        if self.pending.is_some() {
            self.pending.as_mut().unwrap().toks.push(Tok::Lit(v));
        } else {
            self.push(v);
        }
    }

    /// Push (interpret) or compile a (c-addr u) pair.
    fn push_or_compile_pair(&mut self, addr: u64, len: u64) {
        if self.pending.is_some() {
            let d = self.pending.as_mut().unwrap();
            d.toks.push(Tok::Lit(addr as i64));
            d.toks.push(Tok::Lit(len as i64));
        } else {
            self.push(addr as i64);
            self.push(len as i64);
        }
    }

    /// `." ` behaviour: type the string now (interpret) or compile a type call.
    fn emit_type(&mut self, addr: u64, len: u64) -> Result<()> {
        if self.pending.is_some() {
            let xt = self.xt_of("type_word")?;
            let d = self.pending.as_mut().unwrap();
            d.toks.push(Tok::Lit(addr as i64));
            d.toks.push(Tok::Lit(len as i64));
            d.toks.push(Tok::Call(xt));
        } else {
            self.push(addr as i64);
            self.push(len as i64);
            self.call("type_word")?;
        }
        Ok(())
    }

    pub fn eval(&mut self, text: &str) -> Result<()> {
        // Cursor-based tokenizer (not split_whitespace) so string-parsing words
        // can read exact spans with interior spacing preserved.
        let b = text.as_bytes();
        let mut pos = 0usize;
        while let Some(tok) = scan_word(b, &mut pos) {
            // Comments: `\` to end of line, `( … )` to the closing paren.
            if tok == "\\" {
                while pos < b.len() && b[pos] != b'\n' {
                    pos += 1;
                }
                continue;
            }
            if tok == "(" {
                while pos < b.len() && b[pos] != b')' {
                    pos += 1;
                }
                if pos < b.len() {
                    pos += 1; // skip the ')'
                }
                continue;
            }
            let lk = tok.to_ascii_lowercase(); // Forth is case-insensitive
            // CREATE/DOES> defining words: a named `: name … create … [does> …] ;`
            // captures its body raw (replayed at use time) rather than compiling it.
            // Named-only (not :noname / :m), and `create` may appear anywhere.
            if self.pending.is_some() {
                let def = self.pending.as_ref().unwrap();
                let named = !def.noname && self.method_class.is_none();
                if def.defining {
                    if lk == ";" {
                        self.finish_defining()?;
                    } else {
                        self.pending.as_mut().unwrap().raw.push(tok.to_string());
                    }
                    continue;
                }
                if named {
                    if lk == "create" {
                        let d = self.pending.as_mut().unwrap();
                        d.defining = true;
                        d.raw.push("create".to_string());
                        continue;
                    }
                    // record raw for potential later detection, then compile normally
                    self.pending.as_mut().unwrap().raw.push(tok.to_string());
                }
            }
            // A registered defining word: define the next name from its template.
            if self.pending.is_none() {
                if let Some((build, does_xt)) = self.defining_words.get(&lk).cloned() {
                    let target = scan_word(b, &mut pos)
                        .ok_or_else(|| anyhow::anyhow!("`{tok}` needs a name"))?;
                    self.instantiate_defining(target, &build, does_xt)?;
                    continue;
                }
            }
            // [if] / [else] / [then] — conditional compilation (token skipping).
            if lk == "[if]" {
                let flag = self.pop_data().unwrap_or(0);
                if flag == 0 {
                    // skip the true-branch up to the matching [else] or [then]
                    let mut depth = 0;
                    while let Some(t) = scan_word(b, &mut pos) {
                        match t.to_ascii_lowercase().as_str() {
                            "[if]" => depth += 1,
                            "[else]" if depth == 0 => break,
                            "[then]" => {
                                if depth == 0 {
                                    break;
                                }
                                depth -= 1;
                            }
                            _ => {}
                        }
                    }
                }
                continue;
            }
            if lk == "[else]" {
                // reached after a taken true-branch: skip to the matching [then]
                let mut depth = 0;
                while let Some(t) = scan_word(b, &mut pos) {
                    match t.to_ascii_lowercase().as_str() {
                        "[if]" => depth += 1,
                        "[then]" => {
                            if depth == 0 {
                                break;
                            }
                            depth -= 1;
                        }
                        _ => {}
                    }
                }
                continue;
            }
            if lk == "[then]" {
                continue;
            }
            // [ … ] literal — compile-time interpretation inside a definition.
            if lk == "[" && self.pending.is_some() {
                self.bracket_interpret = true;
                continue;
            }
            if lk == "]" {
                self.bracket_interpret = false;
                continue;
            }
            if lk == "literal" && self.pending.is_some() {
                let v = self
                    .pop_data()
                    .ok_or_else(|| anyhow::anyhow!("`literal` needs a value"))?;
                self.pending.as_mut().unwrap().toks.push(Tok::Lit(v));
                continue;
            }
            // ' / ['] name — push (interpret) or compile (literal) the word's xt.
            if lk == "'" || lk == "[']" {
                let name = scan_word(b, &mut pos)
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
            // String-parsing words — read exact spans from the source (interior
            // spacing preserved):
            //   s" ccc"   ( -- c-addr u )   push/compile a string literal
            //   ." ccc"   ( -- )            print (interpret) / compile `type`
            //   c" ccc"   ( -- c-addr )     counted-string literal
            //   s\" ccc"  ( -- c-addr u )   escaped string (\n \t \xHH …)
            //   .( ccc)   ( -- )            print immediately
            if lk == "s\"" || lk == ".\"" || lk == "c\"" {
                let body = scan_span(b, &mut pos, b'"')
                    .ok_or_else(|| anyhow::anyhow!("unterminated {tok}"))?;
                let bytes = body.as_bytes().to_vec();
                if lk == "c\"" {
                    let addr = self.store_counted(&bytes);
                    self.push_or_compile_lit(addr as i64);
                } else {
                    let (addr, len) = self.store_string(&bytes);
                    if lk == ".\"" {
                        self.emit_type(addr, len)?;
                    } else {
                        self.push_or_compile_pair(addr, len);
                    }
                }
                continue;
            }
            if lk == "s\\\"" {
                let bytes = scan_escaped(b, &mut pos)
                    .ok_or_else(|| anyhow::anyhow!("unterminated s\\\""))?;
                let (addr, len) = self.store_string(&bytes);
                self.push_or_compile_pair(addr, len);
                continue;
            }
            if lk == ".(" {
                let body = scan_span(b, &mut pos, b')')
                    .ok_or_else(|| anyhow::anyhow!("unterminated .("))?;
                let bytes = body.as_bytes().to_vec();
                let (addr, len) = self.store_string(&bytes); // .( always prints now
                self.push(addr as i64);
                self.push(len as i64);
                self.call("type_word")?;
                continue;
            }
            // char / [char] NAME — push (or compile) the first byte of NAME.
            if lk == "char" || lk == "[char]" {
                let w = scan_word(b, &mut pos)
                    .ok_or_else(|| anyhow::anyhow!("`{tok}` needs a character"))?;
                let c = w.bytes().next().unwrap_or(0) as i64;
                self.push_or_compile_lit(c);
                continue;
            }
            // Value-family stores: `is NAME` (defer), `2to NAME` (2value),
            // `+to NAME` (value). Each pushes the target address then a store op,
            // in interpret or compile state.
            if lk == "is" || lk == "2to" || lk == "+to" {
                let name = scan_word(b, &mut pos)
                    .ok_or_else(|| anyhow::anyhow!("`{tok}` needs a name"))?;
                let (addr, op): (u64, &str) = match lk.as_str() {
                    "+to" => (
                        *self
                            .values
                            .get(name)
                            .ok_or_else(|| anyhow::anyhow!("`+to {name}` — not a value"))?,
                        "plus_store",
                    ),
                    "2to" => (
                        *self
                            .instance_addrs
                            .get(name)
                            .ok_or_else(|| anyhow::anyhow!("`2to {name}` — not a 2value"))?,
                        "two_store",
                    ),
                    _ => (
                        *self
                            .instance_addrs
                            .get(name)
                            .ok_or_else(|| anyhow::anyhow!("`is {name}` — not a deferred word"))?,
                        "store",
                    ),
                };
                if self.pending.is_some() {
                    let xt = self.xt_of(op)?;
                    let d = self.pending.as_mut().unwrap();
                    d.toks.push(Tok::Lit(addr as i64));
                    d.toks.push(Tok::Call(xt));
                } else {
                    self.push(addr as i64);
                    self.call(op)?;
                }
                continue;
            }
            // synonym NEW OLD — define NEW to execute OLD.
            if lk == "synonym" {
                let newname = scan_word(b, &mut pos)
                    .ok_or_else(|| anyhow::anyhow!("`synonym` needs a new name"))?
                    .to_string();
                let oldname = scan_word(b, &mut pos)
                    .ok_or_else(|| anyhow::anyhow!("`synonym` needs an old name"))?;
                let xt = self
                    .xt_of_forth_name(oldname)?
                    .ok_or_else(|| anyhow::anyhow!("`synonym`: `{oldname}` is undefined"))?;
                self.push_name(&newname);
                self.call("create")?;
                let header = self.read_user(USER_LATEST);
                self.write_u64(header + DH_XTPTR, xt);
                self.write_u8(header + DH_TFA, TFA_TCOL);
                continue;
            }
            // marker NAME — snapshot the dictionary, define NAME as a findable
            // placeholder; executing NAME (intercepted in interpret_token) rolls
            // the dictionary back to here, forgetting NAME and all later defs.
            if lk == "marker" {
                let name = scan_word(b, &mut pos)
                    .ok_or_else(|| anyhow::anyhow!("`marker` needs a name"))?
                    .to_ascii_lowercase();
                let snap = self.snapshot();
                self.publish_constant(&name, 0)?;
                self.markers.insert(name, snap);
                continue;
            }
            // forget NAME — forget NAME and everything defined after it.
            if lk == "forget" {
                let name = scan_word(b, &mut pos)
                    .ok_or_else(|| anyhow::anyhow!("`forget` needs a name"))?
                    .to_ascii_lowercase();
                self.forget_word(&name)?;
                continue;
            }
            // Receiver tracking for early binding: any token other than `->`
            // begins a fresh receiver context.
            if lk != "->" {
                self.last_receiver_class = None;
            }
            // OOP method send — unless `->` has been defined as an ordinary word
            // (e.g. the ANS tester's result separator), which then takes priority.
            if lk == "->" && self.xt_of_forth_name("->")?.is_none() {
                let s = scan_word(b, &mut pos).ok_or_else(|| anyhow::anyhow!("`->` needs a selector"))?;
                self.oop_send(s)?;
                continue;
            }
            // ── OOP parsing words (valid in both interpret and compile state) ──
            match lk.as_str() {
                ":m" => {
                    let s = scan_word(b, &mut pos).ok_or_else(|| anyhow::anyhow!("`:m` needs a name"))?;
                    self.oop_begin_method(s)?;
                    continue;
                }
                ";m" => {
                    self.oop_end_method()?;
                    continue;
                }
                "ivar:" => {
                    let s = scan_word(b, &mut pos).ok_or_else(|| anyhow::anyhow!("`ivar:` needs a name"))?;
                    self.oop_ivar(s)?;
                    continue;
                }
                "end-class" => {
                    self.oop_end_class()?;
                    continue;
                }
                "class" => {
                    let s = scan_word(b, &mut pos).ok_or_else(|| anyhow::anyhow!("`class` needs a name"))?;
                    self.oop_start_class(s, "object")?;
                    continue;
                }
                "subclass" => {
                    let s = scan_word(b, &mut pos).ok_or_else(|| anyhow::anyhow!("`subclass` needs a name"))?;
                    let parent = self.pop_data().unwrap_or(0) as u64;
                    let pn = self
                        .class_name_by_struct(parent)
                        .ok_or_else(|| anyhow::anyhow!("`subclass`: parent is not a class"))?;
                    self.oop_start_class(s, &pn)?;
                    continue;
                }
                "new" => {
                    let s = scan_word(b, &mut pos).ok_or_else(|| anyhow::anyhow!("`new` needs a name"))?;
                    let cs = self.pop_data().unwrap_or(0) as u64;
                    let cn = self
                        .class_name_by_struct(cs)
                        .ok_or_else(|| anyhow::anyhow!("`new`: not a class"))?;
                    self.oop_new(&cn, s)?;
                    continue;
                }
                "create" => {
                    let s = scan_word(b, &mut pos).ok_or_else(|| anyhow::anyhow!("`create` needs a name"))?;
                    let addr = self.read_user(USER_VAR_HERE);
                    self.publish_constant(s, addr as i64)?; // NAME pushes the data field addr
                    self.var_addrs.insert(s.to_string(), addr); // fold to a literal in code
                    self.instance_addrs.insert(s.to_string(), addr);
                    continue;
                }
                "[defined]" => {
                    let s = scan_word(b, &mut pos).ok_or_else(|| anyhow::anyhow!("`[defined]` needs a name"))?;
                    self.oop_defined_q(s)?;
                    continue;
                }
                "[undefined]" => {
                    let s = scan_word(b, &mut pos)
                        .ok_or_else(|| anyhow::anyhow!("`[undefined]` needs a name"))?;
                    self.oop_defined_q(s)?; // -1 if defined, 0 if not
                    let v = self.pop_data().unwrap_or(0);
                    self.push(if v == 0 { -1 } else { 0 });
                    continue;
                }
                "object?" => {
                    self.oop_object_q();
                    continue;
                }
                "class?" => {
                    self.oop_class_q();
                    continue;
                }
                "is-a?" => {
                    self.oop_is_a_q();
                    continue;
                }
                ".class" => {
                    self.oop_dot_class();
                    continue;
                }
                _ => {}
            }
            // ── Class / object names used as operands ──
            if let Some(c) = self.classes.get(tok) {
                let addr = c.struct_addr as i64;
                if self.pending.is_some() {
                    self.pending.as_mut().unwrap().toks.push(Tok::Lit(addr));
                } else {
                    self.push(addr);
                }
                continue;
            }
            if let Some(&(addr, ref cls)) = self.objects.get(tok) {
                let cls = cls.clone();
                if self.pending.is_some() {
                    self.pending.as_mut().unwrap().toks.push(Tok::Lit(addr as i64));
                } else {
                    self.push(addr as i64);
                }
                self.last_receiver_class = Some(cls);
                continue;
            }
            if self.pending.is_some() && !self.bracket_interpret {
                if lk == ";" {
                    self.finish_colon()?;
                } else if lk == "{:" {
                    // {: in0 in1 … | uninit … :}  — declare a locals frame
                    let mut names = Vec::new();
                    let mut inputs = 0usize;
                    let mut after_pipe = false;
                    while let Some(t) = scan_word(b, &mut pos) {
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
                    let target = scan_word(b, &mut pos)
                        .ok_or_else(|| anyhow::anyhow!("`to` needs a name"))?;
                    self.compile_to(target)?;
                } else {
                    self.compile_token(tok)?;
                }
            } else if lk == "bye" {
                self.bye = true;
                break;
            } else if lk == "value" {
                let name = scan_word(b, &mut pos)
                    .ok_or_else(|| anyhow::anyhow!("`value` needs a name"))?;
                let v = self
                    .pop_data()
                    .ok_or_else(|| anyhow::anyhow!("`value` needs an initial value"))?;
                let cell = self.allot_cell();
                self.write_u64(cell, v as u64);
                self.publish_value(name, cell)?;
                self.values.insert(name.to_string(), cell);
            } else if lk == "to" && self.pending.is_none() {
                // interpret-mode `to NAME` — store TOS into a value's cell
                let name = scan_word(b, &mut pos).ok_or_else(|| anyhow::anyhow!("`to` needs a name"))?;
                let cell = *self
                    .values
                    .get(name)
                    .ok_or_else(|| anyhow::anyhow!("`to {name}` — not a value"))?;
                let v = self.pop_data().ok_or_else(|| anyhow::anyhow!("`to` needs a value"))?;
                self.write_u64(cell, v as u64);
            } else if lk == ":" || lk == ":noname" {
                let name = if lk == ":noname" {
                    String::new()
                } else {
                    scan_word(b, &mut pos)
                        .ok_or_else(|| anyhow::anyhow!("`:` needs a name"))?
                        .to_string()
                };
                let mut body = Vec::new();
                crate::aenc::emit_nest(&mut body);
                let self_xt = self.code.next_addr();
                self.pending = Some(ColonDef {
                    name,
                    body,
                    cf: Vec::new(),
                    toks: Vec::new(),
                    locals: Vec::new(),
                    noname: lk == ":noname",
                    self_xt,
                    defining: false,
                    raw: Vec::new(),
                    exits: Vec::new(),
                    metrics: Default::default(),
                });
            } else if lk == "2constant" {
                let name = scan_word(b, &mut pos)
                    .ok_or_else(|| anyhow::anyhow!("`2constant` needs a name"))?;
                let hi = self.pop_data().unwrap_or(0);
                let lo = self.pop_data().unwrap_or(0);
                self.publish_dconstant(name, lo, hi)?;
            } else {
                self.interpret_token(tok)?;
            }
        }
        Ok(())
    }

    /// Capture the current dictionary/data/code state for a `marker`.
    fn snapshot(&self) -> Marker {
        Marker {
            latest: self.read_user(USER_LATEST),
            here: self.read_user(USER_HERE),
            var_here: self.read_user(USER_VAR_HERE),
            index_here: self.read_user(USER_INDEX_HERE),
            index_latest: self.read_user(USER_INDEX_LATEST),
            code_cursor: self.code.checkpoint(),
            classes: self.classes.clone(),
            objects: self.objects.clone(),
            values: self.values.clone(),
            var_addrs: self.var_addrs.clone(),
            instance_addrs: self.instance_addrs.clone(),
            defining_words: self.defining_words.clone(),
        }
    }

    /// Peel every overlay node created after `threshold` (a marker's index_here)
    /// out of its hash bucket, walking the global creation chain newest-first,
    /// then restore the index pointers.
    fn rollback_overlay(&mut self, threshold: u64, index_latest: u64) {
        let mut node = self.read_user(USER_INDEX_LATEST);
        while node != 0 && node < threshold {
            let bucket_next = self.read_u64(node + DN_BUCKET_NEXT);
            let hash = self.read_u64(node + DN_HASH) & 0xFFFF_FFFF;
            let wid = self.read_u64(node + DN_WID);
            let bucket = wid + ((hash & WL_BUCKET_MASK) << 3);
            if self.read_u64(bucket) == node {
                self.write_u64(bucket, bucket_next);
            }
            node = self.read_u64(node + DN_GLOBAL_PREV);
        }
        self.write_user(USER_INDEX_LATEST, index_latest);
        self.write_user(USER_INDEX_HERE, threshold);
    }

    /// Execute a marker: restore the dictionary to its snapshot, forgetting every
    /// definition (and any later markers) made after it.
    fn run_marker(&mut self, m: Marker) {
        self.rollback_overlay(m.index_here, m.index_latest);
        self.write_user(USER_LATEST, m.latest);
        self.write_user(USER_HERE, m.here);
        self.write_user(USER_VAR_HERE, m.var_here);
        self.code.reset_to(m.code_cursor);
        self.classes = m.classes;
        self.objects = m.objects;
        self.values = m.values;
        self.var_addrs = m.var_addrs;
        self.instance_addrs = m.instance_addrs;
        self.defining_words = m.defining_words;
        let t = m.index_here;
        self.markers.retain(|_, mk| mk.index_here > t); // drop markers made after
    }

    /// forget NAME — forget NAME and every definition made after it (no snapshot;
    /// reconstruct the rollback point from NAME's overlay node + header).
    fn forget_word(&mut self, name: &str) -> Result<()> {
        self.push_name(name);
        self.call("find_name")?;
        if self.stack().first() != Some(&-1) {
            self.call("drop_")?;
            self.call("drop_")?;
            self.call("drop_")?;
            anyhow::bail!("forget: `{name}` not found");
        }
        self.call("drop_")?; // drop the -1
        let nt = self.stack()[0] as u64;
        self.call("drop_")?; // drop the nt
        let header = nt - DH_NT;
        // locate NAME's overlay node (global creation chain, newest-first)
        let mut node = self.read_user(USER_INDEX_LATEST);
        while node != 0 && self.read_u64(node + DN_HEADER) != header {
            node = self.read_u64(node + DN_GLOBAL_PREV);
        }
        if node == 0 {
            anyhow::bail!("forget: no overlay node for `{name}`");
        }
        let prev = self.read_u64(node + DN_GLOBAL_PREV);
        let link = self.read_u64(header + DH_LINK);
        self.rollback_overlay(node + DN_SIZE, prev);
        self.write_user(USER_LATEST, link);
        self.write_user(USER_HERE, header);
        self.prune_maps();
        Ok(())
    }

    /// Drop Rust-side map entries whose name is no longer in the dictionary
    /// (after a `forget`, which doesn't carry a snapshot of the maps).
    fn prune_maps(&mut self) {
        macro_rules! prune {
            ($m:ident) => {{
                let keys: Vec<String> = self.$m.keys().cloned().collect();
                for k in keys {
                    if self.xt_of_forth_name(&k).ok().flatten().is_none() {
                        self.$m.remove(&k);
                    }
                }
            }};
        }
        prune!(values);
        prune!(var_addrs);
        prune!(instance_addrs);
        prune!(defining_words);
        prune!(classes);
        prune!(objects);
        prune!(markers);
    }

    /// Interpret one token: find+execute, else number→push, else error.
    fn interpret_token(&mut self, tok: &str) -> Result<()> {
        let lk = tok.to_ascii_lowercase();
        if let Some(m) = self.markers.remove(&lk) {
            self.run_marker(m);
            return Ok(());
        }
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
        // `to <ivar>` inside a method, else `to <local>`, else `to <value>`.
        if let Some(off) = self.ivar_offset(target) {
            self.pending.as_mut().unwrap().toks.push(Tok::IvarStore(off));
            return Ok(());
        }
        if let Some(&cell) = self.values.get(target) {
            // store TOS to the value's cell: ( x -- ) emit Lit(cell), Store
            let d = self.pending.as_mut().unwrap();
            d.toks.push(Tok::Lit(cell as i64));
            d.toks.push(Tok::Mem(crate::opt::Mem::Store));
            return Ok(());
        }
        let def = self.pending.as_ref().unwrap();
        match def.locals.iter().position(|n| n.eq_ignore_ascii_case(target)) {
            Some(i) => {
                self.pending.as_mut().unwrap().toks.push(Tok::LocalStore(i as u32));
                Ok(())
            }
            None => anyhow::bail!("`to {target}` — not a local/ivar"),
        }
    }

    fn compile_token(&mut self, tok: &str) -> Result<()> {
        // A variable/buffer name folds to its fixed address (a literal), so the
        // optimizer can keep it in a register and `@`/`!` become a plain ldr/str
        // with no veneer call.
        if let Some(&addr) = self.var_addrs.get(tok) {
            self.pending.as_mut().unwrap().toks.push(Tok::Lit(addr as i64));
            return Ok(());
        }
        // A bare ivar name (inside a method body) compiles to a SELF-relative fetch.
        if let Some(off) = self.ivar_offset(tok) {
            self.pending.as_mut().unwrap().toks.push(Tok::IvarFetch(off));
            return Ok(());
        }
        // `recurse` — compile a call to the definition currently being built.
        if tok.eq_ignore_ascii_case("recurse") {
            let xt = self.pending.as_ref().unwrap().self_xt;
            self.pending.as_mut().unwrap().toks.push(Tok::Call(xt));
            return Ok(());
        }
        // `super` inside a method: push self and mark the next `->` as a super send.
        if tok.eq_ignore_ascii_case("super") {
            self.pending.as_mut().unwrap().toks.push(Tok::SelfPush);
            self.super_pending = true;
            return Ok(());
        }
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
        // Constant-index pick/roll: `N pick` / `N roll` with a literal N becomes
        // static stack motion in the window (no runtime index, no kernel call) —
        // the "everything is pick" model made concrete.
        if matches!(lk.as_str(), "pick" | "roll") {
            if let Some(Tok::Lit(n)) = self.pending.as_ref().and_then(|d| d.toks.last()).copied() {
                if (0..=6).contains(&n) {
                    self.pending.as_mut().unwrap().toks.pop(); // drop the literal index
                    let t = if lk == "pick" {
                        Tok::PickN(n as u32)
                    } else {
                        Tok::RollN(n as u32)
                    };
                    self.pending.as_mut().unwrap().toks.push(t);
                    return Ok(());
                }
            }
        }
        // `unless` ≡ `0= if` — push a zero-test, then compile as `if` (the cmp
        // fuses into a single inverted branch).
        if lk == "unless" {
            self.pending
                .as_mut()
                .unwrap()
                .toks
                .push(Tok::Cmp(crate::opt::Cmp::ZEq));
            return self.compile_token("if");
        }
        // CASE / OF / ENDOF / ENDCASE — expand to if/else/then:
        //   of ≡ `over = if drop`,  endof ≡ `else`,  endcase ≡ `drop` + close
        //   every open `then` back to the CASE marker.
        if lk == "case" {
            self.pending.as_mut().unwrap().cf.push(Cf::Case);
            return Ok(());
        }
        if lk == "of" {
            let d = self.pending.as_mut().unwrap();
            d.toks.push(Tok::Stk(crate::opt::Stk::Over));
            d.toks.push(Tok::Cmp(crate::opt::Cmp::Eq));
            self.compile_token("if")?; // over = fuses into the branch
            self.pending.as_mut().unwrap().toks.push(Tok::Stk(crate::opt::Stk::Drop)); // matched: drop selector
            return Ok(());
        }
        if lk == "endof" {
            return self.compile_token("else");
        }
        if lk == "endcase" {
            self.pending.as_mut().unwrap().toks.push(Tok::Stk(crate::opt::Stk::Drop)); // default: drop selector
            self.flush_toks();
            loop {
                match self.pending.as_ref().unwrap().cf.last() {
                    Some(Cf::Case) => {
                        self.pending.as_mut().unwrap().cf.pop();
                        return Ok(());
                    }
                    Some(_) => self.compile_control("then")?,
                    None => anyhow::bail!("`endcase` without `case`"),
                }
            }
        }
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
            "if" | "else" | "then" | "begin" | "until" | "while" | "repeat" | "do" | "loop"
                | "+loop" | "?do" | "leave" | "unloop" | "exit"
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
                    // FLit pushes onto the optimizer's FP virtual stack directly.
                    self.pending.as_mut().unwrap().toks.push(Tok::FLit(f.to_bits() as i64));
                }
            },
        }
        Ok(())
    }

    /// A formatted optimizer-effectiveness report over every compiled word.
    /// Frames the deferred-lowerer's behavior as: token reduction, register/stack
    /// residency, and the heat-policy gap (hot values re-read from memory or
    /// evicted by coarse spills — what a hotness-ranked allocator would capture).
    pub fn optimizer_report(&self) -> String {
        use core::fmt::Write as _;
        let mut total = crate::opt::Metrics::default();
        for m in self.word_metrics.values() {
            total.merge(m); // counts sum; peaks take the max single-word pressure
        }
        let n = self.word_metrics.len();
        let pct = |a: u32, b: u32| if b == 0 { 0.0 } else { 100.0 * a as f64 / b as f64 };
        let mut s = String::new();
        let _ = writeln!(s, "═══ MF66 optimizer report — {n} words ═══");
        let _ = writeln!(s, "\nToken reduction (reduce pass):");
        let _ = writeln!(
            s,
            "  tokens {} → {}  ({:.0}% removed)",
            total.toks_in,
            total.toks_out,
            pct(total.toks_in.saturating_sub(total.toks_out), total.toks_in)
        );
        let _ = writeln!(
            s,
            "  const-fold {}  imm-fold {}  imm-chain {}  dup-fuse {}  DCE {}  stk-cancel {}  cmp-neg {}  (total {} rewrites)",
            total.const_folds, total.imm_folds, total.imm_chains, total.dup_fuses,
            total.dce, total.stack_cancels, total.cmp_negates, total.rewrites()
        );
        let _ = writeln!(s, "\nRegisters & stack motion:");
        let _ = writeln!(
            s,
            "  stack ops {}  free/no-code {}  ({:.0}% pure reindex)",
            total.stack_ops,
            total.stack_ops_free,
            pct(total.stack_ops_free, total.stack_ops)
        );
        let _ = writeln!(
            s,
            "  GP allocs {}  peak GP {}/7  peak FP {}/7  reg-copies {}  const→reg {}",
            total.gp_allocs, total.peak_gp, total.peak_fp, total.reg_copies, total.const_mat
        );
        let _ = writeln!(s, "  spills (pressure → full-window settle) {}", total.spills);
        let _ = writeln!(s, "\nMemory traffic:");
        let _ = writeln!(
            s,
            "  stack loads {}  stack stores {}  DSP adjusts {}  settles {}  calls {}",
            total.stack_loads, total.stack_stores, total.dsp_adjusts, total.settles, total.calls
        );
        let _ = writeln!(s, "  data @ {}  ! {}", total.mem_fetches, total.mem_stores);
        let _ = writeln!(
            s,
            "  redundant @-refetches {}  ← heat-policy gap (hot value re-read from memory, not kept in a reg)",
            total.redundant_fetches
        );
        // The hot words: most heat-gap (redundant fetches + spills), then biggest.
        let mut v: Vec<(&String, &crate::opt::Metrics)> = self.word_metrics.iter().collect();
        v.sort_by(|a, b| {
            let ga = a.1.redundant_fetches + a.1.spills;
            let gb = b.1.redundant_fetches + b.1.spills;
            gb.cmp(&ga).then(b.1.instrs.cmp(&a.1.instrs))
        });
        let _ = writeln!(s, "\nHeat-gap words (refetch+spill, then size):");
        let _ = writeln!(
            s,
            "  {:<18} {:>5} {:>8} {:>6} {:>6} {:>5} {:>4}",
            "word", "instrs", "refetch", "spills", "peakGP", "loads", "@"
        );
        for (name, m) in v.iter().take(14) {
            if m.redundant_fetches + m.spills + m.mem_fetches == 0 && m.peak_gp <= 1 {
                continue;
            }
            let _ = writeln!(
                s,
                "  {:<18} {:>5} {:>8} {:>6} {:>6} {:>5} {:>4}",
                name, m.instrs, m.redundant_fetches, m.spills, m.peak_gp, m.stack_loads, m.mem_fetches
            );
        }
        s
    }

    /// Reduce + lower the accumulated token run into the body, then clear it.
    /// Optimizer metrics accumulate into the pending definition.
    fn flush_toks(&mut self) {
        if let Some(def) = self.pending.as_mut() {
            if !def.toks.is_empty() {
                let reduced = crate::opt::reduce(&def.toks, &mut def.metrics);
                crate::opt::lower(&reduced, &mut def.body, &mut def.metrics);
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
            ("rot_word", &[Tok::Stk(Rot)]),
            ("minus_rot_word", &[Tok::Stk(MinusRot)]),
            ("tuck_word", &[Tok::Stk(Tuck)]),
            // Floating point — modeled in the FP virtual stack.
            ("f_plus", &[Tok::FBin(crate::opt::FBin::Add)]),
            ("f_minus", &[Tok::FBin(crate::opt::FBin::Sub)]),
            ("f_times", &[Tok::FBin(crate::opt::FBin::Mul)]),
            ("f_slash", &[Tok::FBin(crate::opt::FBin::Div)]),
            ("f_negate", &[Tok::FUn(crate::opt::FUn::Neg)]),
            ("fsqrt_word", &[Tok::FUn(crate::opt::FUn::Sqrt)]),
            ("fabs_word", &[Tok::FUn(crate::opt::FUn::Abs)]),
            ("fdup", &[Tok::FStk(Dup)]),
            ("fdrop", &[Tok::FStk(Drop)]),
            ("fswap", &[Tok::FStk(Swap)]),
            ("fover", &[Tok::FStk(Over)]),
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
            ("min_", &[Tok::Sel(crate::opt::Sel::Min)]),
            ("max_", &[Tok::Sel(crate::opt::Sel::Max)]),
            ("umin", &[Tok::Sel(crate::opt::Sel::UMin)]),
            ("umax", &[Tok::Sel(crate::opt::Sel::UMax)]),
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
                def.cf.push(Cf::Do(def.body.len(), Vec::new())); // loop body top
            }
            "?do" => {
                // set up the frame, skip the body if index == limit
                let skip = crate::aenc::emit_qdo(&mut def.body);
                def.cf.push(Cf::Do(def.body.len(), vec![skip]));
            }
            "leave" => {
                let i = crate::aenc::emit_leave(&mut def.body);
                let fwds = def
                    .cf
                    .iter_mut()
                    .rev()
                    .find_map(|c| match c {
                        Cf::Do(_, f) => Some(f),
                        _ => None,
                    })
                    .ok_or_else(|| anyhow::anyhow!("`leave` outside a do-loop"))?;
                fwds.push(i);
            }
            "unloop" => {
                crate::aenc::emit_unloop(&mut def.body);
            }
            "exit" => {
                // early return: branch to the word's single epilogue (which runs
                // the locals-frame teardown + unnest + ret). Caller must `unloop`
                // first if inside a do-loop, per ANS.
                let i = def.body.len();
                def.body.push(crate::aenc::b(0));
                def.exits.push(i);
            }
            "loop" => {
                let (top, fwds) = match def.cf.pop() {
                    Some(Cf::Do(t, f)) => (t, f),
                    _ => anyhow::bail!("`loop` without `do`"),
                };
                crate::aenc::emit_loop(&mut def.body, top);
                let exit = def.body.len() - 1; // the frame-drop `add RP,RP,16`
                for i in fwds {
                    crate::aenc::patch_bcond(&mut def.body, i, exit);
                }
            }
            "+loop" => {
                let (top, fwds) = match def.cf.pop() {
                    Some(Cf::Do(t, f)) => (t, f),
                    _ => anyhow::bail!("`+loop` without `do`"),
                };
                crate::aenc::emit_plus_loop(&mut def.body, top);
                let exit = def.body.len() - 1;
                for i in fwds {
                    crate::aenc::patch_bcond(&mut def.body, i, exit);
                }
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
    /// Finish the pending body (flush, free locals, unnest+ret) and commit it to
    /// the code arena, returning its xt. Does NOT publish a header.
    fn commit_body(&mut self) -> Result<u64> {
        self.flush_toks(); // lower the final straight-line run
        let mut def = self.pending.take().expect("commit_body with no pending def");
        if !def.cf.is_empty() {
            anyhow::bail!("unbalanced control flow in `{}`", def.name);
        }
        // The single epilogue: every `exit` branches here so the locals teardown
        // and unnest+ret exist once, not inlined at each early return.
        let epilogue = def.body.len();
        if !def.locals.is_empty() {
            def.body.push(crate::aenc::add_imm(21, 21, (def.locals.len() * 8) as u32));
        }
        crate::aenc::emit_unnest_ret(&mut def.body);
        let exits = std::mem::take(&mut def.exits);
        for i in exits {
            crate::aenc::patch_b(&mut def.body, i, epilogue);
        }
        self.last_body_words = def.body.len();
        // Finalize + record optimizer metrics for this definition (body words
        // less the nest prologue + the single epilogue).
        def.metrics.instrs = def.body.len().saturating_sub(2) as u32;
        if !def.name.is_empty() {
            self.word_metrics.insert(def.name.clone(), def.metrics.clone());
        }
        self.code.commit(&def.body)
    }

    fn finish_colon(&mut self) -> Result<()> {
        let def = self.pending.as_ref().expect("finish_colon: no def");
        let name = def.name.clone();
        let noname = def.noname;
        let xt = self.commit_body()?;
        if noname {
            self.push(xt as i64); // `:noname … ;` leaves the xt
            return Ok(());
        }
        // Header (in the RW dict heap) via (create), then patch xt + type tag.
        self.push_name(&name);
        self.call("create")?;
        let header = self.read_user(USER_LATEST);
        self.write_u64(header + DH_XTPTR, xt);
        self.write_u8(header + DH_TFA, TFA_TCOL);
        Ok(())
    }

    /// Finish a CREATE/DOES> defining word: split its captured tokens at `does>`,
    /// compile the behavior to a routine, and register it for use-time replay.
    fn finish_defining(&mut self) -> Result<()> {
        let def = self.pending.take().expect("finish_defining: no def");
        let name = def.name;
        let raw = def.raw;
        let does_at = raw.iter().position(|t| t.eq_ignore_ascii_case("does>"));
        let (build, does_xt) = match does_at {
            Some(i) => {
                let build = raw[..i].to_vec();
                let behavior = raw[i + 1..].to_vec();
                let xt = self.compile_routine(&behavior)?;
                (build, xt)
            }
            None => (raw, 0u64),
        };
        self.defining_words.insert(name, (build, does_xt));
        Ok(())
    }

    /// Compile a token list into a standalone routine; return its xt. Used for a
    /// `does>` behavior, which runs with the instance's body address on the stack.
    fn compile_routine(&mut self, tokens: &[String]) -> Result<u64> {
        let mut body = Vec::new();
        crate::aenc::emit_nest(&mut body);
        let self_xt = self.code.next_addr();
        self.pending = Some(ColonDef {
            name: String::new(),
            body,
            cf: Vec::new(),
            toks: Vec::new(),
            locals: Vec::new(),
            noname: false,
            self_xt,
            defining: false,
            raw: Vec::new(),
            exits: Vec::new(),
            metrics: Default::default(),
        });
        for t in tokens {
            self.compile_token(t)?;
        }
        self.commit_body()
    }

    /// Instantiate a CREATE/DOES> defining word: define `target` with a body that
    /// pushes its data-field address and (if any) runs the does-code, then replay
    /// the build tokens in Rust to fill the data field. (Interpret-time only.)
    fn instantiate_defining(
        &mut self,
        target: &str,
        build: &[String],
        does_xt: u64,
    ) -> Result<()> {
        // Replay the build tokens; when we reach `create`, define `target` with a
        // body that pushes its data-field address and tail-calls the does-code.
        for t in build.to_vec() {
            if t.eq_ignore_ascii_case("create") {
                let body_addr = self.read_user(USER_VAR_HERE);
                self.instance_addrs.insert(target.to_string(), body_addr);
                let mut wbody = Vec::new();
                crate::aenc::emit_lit(body_addr as i64, &mut wbody);
                if does_xt != 0 {
                    // tail-call: the does-code's ret returns to our caller (no nest
                    // here, so a blr+ret would loop on the clobbered x30).
                    crate::aenc::emit_tail_call(does_xt, &mut wbody);
                } else {
                    // no does> → the word pushes a fixed address; fold it inline.
                    self.var_addrs.insert(target.to_string(), body_addr);
                    wbody.push(crate::aenc::ret());
                }
                let xt = self.code.commit(&wbody)?;
                self.push_name(target);
                self.call("create")?;
                let header = self.read_user(USER_LATEST);
                self.write_u64(header + DH_XTPTR, xt);
                self.write_u8(header + DH_TFA, TFA_TCOL);
            } else {
                self.interpret_token(&t)?;
            }
        }
        Ok(())
    }

    /// Publish `name` as a leaf word that pushes a double-cell constant `lo hi`.
    fn publish_dconstant(&mut self, name: &str, lo: i64, hi: i64) -> Result<u64> {
        let mut body = Vec::new();
        crate::aenc::emit_lit(lo, &mut body); // TOS = lo
        crate::aenc::emit_lit(hi, &mut body); // spill lo, TOS = hi → ( lo hi )
        body.push(crate::aenc::ret());
        let xt = self.code.commit(&body)?;
        self.push_name(name);
        self.call("create")?;
        let header = self.read_user(USER_LATEST);
        self.write_u64(header + DH_XTPTR, xt);
        self.write_u8(header + DH_TFA, TFA_TCOL);
        Ok(xt)
    }

    // ── OOP defining words ──────────────────────────────────────────────────

    /// `class N` / `P subclass N`: begin building a class (copy-down from parent).
    fn oop_start_class(&mut self, name: &str, parent: &str) -> Result<()> {
        let p = self
            .classes
            .get(parent)
            .ok_or_else(|| anyhow::anyhow!("unknown parent class `{parent}`"))?;
        self.pending_class = Some(ClassInfo {
            name: name.to_string(),
            super_name: Some(parent.to_string()),
            struct_addr: 0,
            isize_bytes: p.isize_bytes,
            ivars: p.ivars.clone(),
            methods: p.methods.clone(), // inherit (copy-down)
        });
        Ok(())
    }

    /// `<size> ivar: NAME` — add an instance variable to the pending class.
    fn oop_ivar(&mut self, name: &str) -> Result<()> {
        let size = self.pop_data().unwrap_or(CELL as i64) as u64;
        let c = self
            .pending_class
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("`ivar:` outside a class"))?;
        let off = c.isize_bytes;
        c.ivars.push((name.to_string(), off));
        c.isize_bytes += size;
        Ok(())
    }

    /// `end-class` — allocate the data-space class struct and register the class.
    fn oop_end_class(&mut self) -> Result<()> {
        let mut c = self
            .pending_class
            .take()
            .ok_or_else(|| anyhow::anyhow!("`end-class` without `class`"))?;
        // class struct: [+0]=super, [+8]=reserved, [+16]=vtable[VTABLE_CAP]
        let struct_addr = self.read_user(USER_VAR_HERE);
        let total = 16 + VTABLE_CAP * CELL as u64;
        self.write_user(USER_VAR_HERE, struct_addr + total);
        let super_addr = c
            .super_name
            .as_ref()
            .and_then(|p| self.classes.get(p))
            .map(|p| p.struct_addr)
            .unwrap_or(0);
        self.write_u64(struct_addr, super_addr);
        self.write_u64(struct_addr + 8, 0);
        // fill the vtable: every slot = (dnu), then install this class's methods
        for k in 0..VTABLE_CAP {
            self.write_u64(struct_addr + 16 + k * CELL as u64, self.dnu_xt);
        }
        for (sel, xt) in c.methods.clone() {
            let id = self.selector_id(&sel);
            self.write_u64(struct_addr + 16 + id * CELL as u64, xt);
        }
        c.struct_addr = struct_addr;
        self.classes.insert(c.name.clone(), c);
        Ok(())
    }

    /// `C new NAME` — allocate + zero an instance, store its class, register it.
    fn oop_new(&mut self, class_name: &str, obj_name: &str) -> Result<()> {
        let c = self
            .classes
            .get(class_name)
            .ok_or_else(|| anyhow::anyhow!("unknown class `{class_name}`"))?;
        let isize = c.isize_bytes;
        let struct_addr = c.struct_addr;
        let addr = self.read_user(USER_VAR_HERE);
        self.write_user(USER_VAR_HERE, addr + isize);
        for off in (0..isize).step_by(CELL) {
            self.write_u64(addr + off, 0);
        }
        self.write_u64(addr, struct_addr); // [obj+0] = class
        self.objects.insert(obj_name.to_string(), (addr, class_name.to_string()));
        self.publish_constant(obj_name, addr as i64)?; // NAME pushes its base addr
        Ok(())
    }

    /// `:m SELECTOR` — begin compiling a method body in the pending class's scope.
    fn oop_begin_method(&mut self, selector: &str) -> Result<()> {
        let class = self
            .pending_class
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("`:m` outside a class"))?
            .name
            .clone();
        self.selector_id(selector); // assign a stable id
        let mut body = Vec::new();
        crate::aenc::emit_nest(&mut body);
        let self_xt = self.code.next_addr();
        self.pending = Some(ColonDef {
            name: selector.to_string(), // method bodies reuse `name` to hold the selector
            body,
            cf: Vec::new(),
            toks: Vec::new(),
            locals: Vec::new(),
            noname: false,
            self_xt,
            defining: false,
            raw: Vec::new(),
            exits: Vec::new(),
            metrics: Default::default(),
        });
        self.method_class = Some(class);
        Ok(())
    }

    /// `;m` — finish the method body, install its xt in the class's method map.
    fn oop_end_method(&mut self) -> Result<()> {
        let selector = self
            .pending
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("`;m` without `:m`"))?
            .name
            .clone();
        let xt = self.commit_body()?;
        self.method_class = None;
        self.pending_class
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("`;m` outside a class"))?
            .methods
            .insert(selector, xt);
        Ok(())
    }

    /// `recv -> SELECTOR` — compile or execute a method send. The receiver is
    /// already emitted/pushed; `super`/named-object receivers early-bind.
    fn oop_send(&mut self, selector: &str) -> Result<()> {
        let compiling = self.pending.is_some();
        // Resolve an early-bound method xt when the receiver's class is static.
        let early_xt = if self.super_pending {
            // The current method belongs to the class being built (pending_class),
            // which is not yet in `classes`; take its parent from there.
            let parent = self.pending_class.as_ref().and_then(|c| c.super_name.clone());
            parent
                .and_then(|p| self.classes.get(&p).cloned())
                .and_then(|p| p.methods.get(selector).copied())
        } else if let Some(cls) = self.last_receiver_class.clone() {
            self.classes.get(&cls).and_then(|c| c.methods.get(selector).copied())
        } else {
            None
        };
        self.super_pending = false;
        self.last_receiver_class = None;
        if compiling {
            match early_xt {
                Some(xt) => {
                    let send = self.send_xt_xt;
                    let d = self.pending.as_mut().unwrap();
                    d.toks.push(Tok::Lit(xt as i64));
                    d.toks.push(Tok::Call(send));
                }
                None => {
                    let id = self.selector_id(selector) as i64;
                    let send = self.send_xt;
                    let d = self.pending.as_mut().unwrap();
                    d.toks.push(Tok::Lit(id));
                    d.toks.push(Tok::Call(send));
                }
            }
        } else {
            match early_xt {
                Some(xt) => {
                    self.push(xt as i64);
                    self.call("send_xt_word")?;
                }
                None => {
                    let id = self.selector_id(selector);
                    self.push(id as i64);
                    self.call("send_word")?;
                }
            }
        }
        Ok(())
    }

    // ── OOP introspection ───────────────────────────────────────────────────
    fn oop_object_q(&mut self) {
        let obj = self.pop_data().unwrap_or(0) as u64;
        let cls = self.read_u64(obj);
        let is = self.classes.values().any(|c| c.struct_addr != 0 && c.struct_addr == cls);
        self.push(if is { -1 } else { 0 });
    }
    fn oop_class_q(&mut self) {
        let x = self.pop_data().unwrap_or(0) as u64;
        let is = x != 0 && self.classes.values().any(|c| c.struct_addr == x);
        self.push(if is { -1 } else { 0 });
    }
    fn oop_is_a_q(&mut self) {
        let cls = self.pop_data().unwrap_or(0) as u64; // class struct addr
        let obj = self.pop_data().unwrap_or(0) as u64;
        let mut c = self.read_u64(obj); // obj's class
        let mut found = false;
        while c != 0 {
            if c == cls {
                found = true;
                break;
            }
            c = self.read_u64(c); // super at [c+0]
        }
        self.push(if found { -1 } else { 0 });
    }
    fn oop_dot_class(&mut self) {
        let obj = self.pop_data().unwrap_or(0) as u64;
        let cls = self.read_u64(obj);
        if let Some(name) = self.classes.iter().find(|(_, c)| c.struct_addr == cls).map(|(n, _)| n.clone()) {
            crate::runtime::capture_str(&name); // no trailing space; REPL adds ` ok`
        }
    }
    fn oop_defined_q(&mut self, name: &str) -> Result<()> {
        let found = self.find(name)?.is_some()
            || self.classes.contains_key(name)
            || self.objects.contains_key(name);
        self.push(if found { -1 } else { 0 });
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
    /// Abandon any half-finished colon/method definition and clear the stacks —
    /// used to recover after an error when running an external test transcript.
    pub fn reset_input(&mut self) {
        self.pending = None;
        self.method_class = None;
        self.pending_class = None;
        self.super_pending = false;
        self.bracket_interpret = false;
        self.reset();
    }

    pub fn reset(&mut self) {
        self.current_dsp = self.dstack_top;
        self.write_user(USER_DSP_SAVE, self.dstack_top);
        self.write_user(USER_RSP_CURRENT, self.rstack_top);
        self.write_user(UVAR_BASE, 10);
        // OOP: keep the root class + stable selector ids; drop user classes/objects.
        self.classes.retain(|k, _| k == "object");
        self.objects.clear();
        // Keep `values` and `defining_words`: the bootstrap defining words
        // (constant/variable/field:/…) must survive reset like the root class.
        // (User-defined ones leak harmlessly — they are unreferenced after reset.)
        self.pending_class = None;
        self.method_class = None;
        self.super_pending = false;
        self.last_receiver_class = None;
        self.write_user(USER_SELF, 0);
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
    /// Read a cell at an absolute address.
    fn read_u64(&self, addr: u64) -> u64 {
        unsafe { (addr as *const u64).read_unaligned() }
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

/// Next whitespace-delimited word from `b` starting at `*pos`; advances `*pos`
/// past it. `None` at end of input. (The cursor-based replacement for
/// `split_whitespace`, so string-parsing words can read exact spans.)
fn scan_word<'a>(b: &'a [u8], pos: &mut usize) -> Option<&'a str> {
    while *pos < b.len() && b[*pos].is_ascii_whitespace() {
        *pos += 1;
    }
    if *pos >= b.len() {
        return None;
    }
    let start = *pos;
    while *pos < b.len() && !b[*pos].is_ascii_whitespace() {
        *pos += 1;
    }
    std::str::from_utf8(&b[start..*pos]).ok()
}

/// Parse a string body up to `delim`, preserving exact interior spacing. Consumes
/// exactly one leading delimiter space if present (the space that ended the
/// parsing word), then takes bytes verbatim to `delim`. Advances past `delim`.
/// `None` if unterminated. Used by `s"`, `."`, `.(`, `c"`.
fn scan_span<'a>(b: &'a [u8], pos: &mut usize, delim: u8) -> Option<&'a str> {
    if *pos < b.len() && b[*pos] == b' ' {
        *pos += 1;
    }
    let start = *pos;
    while *pos < b.len() && b[*pos] != delim {
        *pos += 1;
    }
    if *pos >= b.len() {
        return None;
    }
    let s = std::str::from_utf8(&b[start..*pos]).ok();
    *pos += 1; // skip the closing delimiter
    s
}

/// Parse an `s\"` escaped string up to an unescaped `"`, decoding ANS 6.2.2266
/// escapes (`\n \t \r \a \b \e \f \l \m \q \v \z \0 \" \\ \xHH`). Consumes one
/// leading space; advances past the closing quote. `None` if unterminated.
fn scan_escaped(b: &[u8], pos: &mut usize) -> Option<Vec<u8>> {
    if *pos < b.len() && b[*pos] == b' ' {
        *pos += 1;
    }
    let hex = |d: u8| -> Option<u8> {
        match d {
            b'0'..=b'9' => Some(d - b'0'),
            b'a'..=b'f' => Some(d - b'a' + 10),
            b'A'..=b'F' => Some(d - b'A' + 10),
            _ => None,
        }
    };
    let mut out = Vec::new();
    while *pos < b.len() {
        let c = b[*pos];
        if c == b'"' {
            *pos += 1;
            return Some(out);
        }
        if c == b'\\' {
            *pos += 1;
            let e = *b.get(*pos)?;
            match e {
                b'a' => out.push(7),
                b'b' => out.push(8),
                b'e' => out.push(27),
                b'f' => out.push(12),
                b'l' => out.push(10),
                b'm' => {
                    out.push(13);
                    out.push(10);
                }
                b'n' => out.push(10),
                b'q' | b'"' => out.push(34),
                b'r' => out.push(13),
                b't' => out.push(9),
                b'v' => out.push(11),
                b'z' | b'0' => out.push(0),
                b'\\' => out.push(92),
                b'x' | b'X' => {
                    let h1 = b.get(*pos + 1).copied().and_then(hex)?;
                    let h2 = b.get(*pos + 2).copied().and_then(hex)?;
                    out.push(h1 * 16 + h2);
                    *pos += 2;
                }
                other => out.push(other), // unknown escape → literal char
            }
            *pos += 1;
        } else {
            out.push(c);
            *pos += 1;
        }
    }
    None
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
    // Forth allows a bare exponent marker: `1e` / `1.5E` mean `…e0`.
    let norm = if tok.ends_with('e') || tok.ends_with('E') {
        format!("{tok}0")
    } else {
        tok.to_string()
    };
    match norm.parse::<f64>() {
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
