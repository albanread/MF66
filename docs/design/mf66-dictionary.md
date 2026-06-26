# MF66 — dictionary & header format (replicating WF66)

Authoritative blueprint for MF66's dictionary, synthesized from a full subsystem
map of WF66 (`docs/review/dict-subsystem-map.json`). The STC compiler's eager
inlining and low-level word typing read the header at compile time, so the
**layout and the type/flag conventions are replicated byte-for-byte**; only the
hand-assembled machine code (the CREATE stub, the compile emitters) is retargeted
to AArch64.

## 1. Two parallel structures (both required)

WF66 maintains two views of every word, and different operations walk different
views — porting only one is insufficient:

1. **Header chain** (`dh_*`, threaded by `dh_link`) — the source of truth, built
   by `(create)`. Walked by `>name`/`>comp`/`>body`/`forget` and the LATEST chain.
2. **Hash overlay** (`dn_*` nodes in a downward arena, hung off 512-bucket
   wordlists) — what `find-name`/`search-wordlist` actually search.

## 2. Header layout — BYTE-IDENTICAL to WF66 (`cell=8`)

Base = address in `LATEST` and in each `dh_link`. These offsets and the `tfa_*`
values are baked into the back-offset arithmetic, `>body`, and (later) `lib/*.f`
literals — **do not change them.**

| field | off | size | meaning |
|---|---|---|---|
| `dh_link` | 0 | 8 | prev header (LATEST chain) |
| `dh_ct` | 8 | 8 | **compile-mode action** = `compile_word` (normal) or `execute` (IMMEDIATE). *This is the immediacy bit — there is no separate flag.* |
| `dh_xtptr` | 16 | 8 | runtime xt (code entry / colon body start) |
| `dh_comp` | 24 | 8 | **compile emitter** = `compile_comma` (emit a call) or an `inline_*`/`fold_*` helper |
| `dh_rec` | 32 | 8 | reserved (0) |
| `dh_vfa` | 40 | 2 | reserved (0) |
| `dh_ofa` | 42 | 2 | **inline body length** (bytes to copy when inlining a leaf; 0 = not inlinable) |
| `dh_stk` | 44 | 2 | stack-effect (reserved, 0; WF66 settles everywhere, reads nothing) |
| `dh_tfa` | 46 | 1 | **word type**: `0`=primitive, `0x82`=colon, `0x91`=create, `0x93`=ivar |
| `dh_nt` | 47 | 1 | name length byte (the **name token** `nt` = this address) |
| `dh_name` | 48 | n | name chars |

Three addresses for "the word": **LATEST/`dh_link`** = header base; **nt** =
base+47 (what `find-name` returns); **xt** = `dh_xtptr` value (code).

**Back-offset cell:** after the name, align to 8 and reserve one cell at `xt-cell`
holding `(dh_ct - xt)` so `>ct`/`>name`/`>comp`/`>body` recover the header from an
xt in O(1) (verified by `[ct+dh_xtptr-dh_ct]==xt`, else fall back to a LATEST scan).
For JIT'd primitives (body in code, not the dict heap) the Rust bootstrap writes
this cell explicitly (`write_primitive_xt_backref`).

## 3. Overlay & hashing — BYTE-IDENTICAL (the hash value is observable)

Overlay node `dn_*`, `dn_size=40`:

| field | off | size |
|---|---|---|
| `dn_bucket_next` | 0 | 8 (collision chain) |
| `dn_global_prev` | 8 | 8 (creation-order LIFO, for forget) |
| `dn_header` | 16 | 8 (→ header base) |
| `dn_wid` | 24 | 8 (owning wordlist) |
| `dn_hash` | 32 | 4 (FNV-1a, dword) |
| `dn_len` | 36 | 1 |
| `dn_first` | 37 | 1 (folded first char) |

Wordlist = 512 buckets × 8 = `wl_size=4096`; bucket = `hash & 511`
(`wl_bucket_mask=511`). `search_order_max=16`.

**Hash = FNV-1a, 32-bit, ASCII-upper-folded** (reject len 0 or >32 first):
```
h = 0x811C9DC5
for each byte c (fold a–z → A–Z by −32; capture folded byte0 as `first`):
    h ^= c;  h = (h * 0x01000193) mod 2^32
bucket = h & 511
```
⚠ **AArch64:** use 32-bit `mul w,w,w` (NOT 64-bit `mul`) so it truncates like
x86 `imul r32`; `ldrb w,[..]` for zero-extended byte loads; **unsigned** branches
(`b.lo/b.hi`) for the fold bounds. The hash, `dn_len`, `dn_first` are a triple
fast-reject before the full **case-insensitive** byte compare (`len` bytes, no
terminator). This must stay byte-identical or names hashed at boot won't be found.

`find-name` walks `CONTEXT[0..ORDER_COUNT)` (first match wins, index 0 first);
publishing always targets `CURRENT`'s wordlist.

## 4. Region & bump pointers

Three independent arenas (extends the user-area layout already adopted, §`macros.masm`):
- **HERE** (`user_HERE`=0x18) — code/header heap, grows **up** from `dict_base`.
- **VAR_HERE** (`user_VAR_HERE`=0x1820) — RW/no-exec data bodies (variables,
  `create` bodies, buffers via `here`/`allot`/`,`), grows **up**. `VAR_LIMIT`=0x1828.
- **INDEX_HERE** (`user_INDEX_HERE`=0x1518) — overlay arena (`dn_*` nodes + wordlist
  bucket tables), grows **down** from `DICT_END` (`user_DICT_END`=0x20).
- Overflow = `INDEX_HERE - size <= HERE` → THROW −8 (the two heaps meet).

Additional user vars to add: `LATESTXT`=0x78, `HANDLER`/`THROW_CODE`/`ROOT_HANDLER`
(already present), `CURRENT`=0x1500, `FORTH_WID`=0x1508, `ORDER_COUNT`=0x1510,
`INDEX_LATEST`=0x1520, `CONTEXT`=0x1528 (16 cells), `TOOLS_WID`=0x17C8,
`PRIVATE_WID`=0x17D0, `VAR_HERE`/`VAR_LIMIT`. (All byte-identical to WF66.)

## 5. Bootstrap protocol — architecture-NEUTRAL (replicate verbatim)

The Rust driver never writes header bytes; it calls kernel primitives. `Mf66Session`
must replicate WF66's sequence exactly:
1. JIT all primitive symbols; zero-init the user area as an empty dict (`LATEST=0`,
   `HERE=dict_base`, `VAR_HERE=var_base`, `DICT_END=…`, `INDEX_HERE=DICT_END`, base=10).
2. `call init_dictionary_overlay` — carves the FORTH wordlist.
3. Allocate TOOLS/PRIVATE wordlists; set `CURRENT` per-word (PRIVATE/TOOLS/FORTH).
4. For each `PRIMITIVES` entry `(forth_name, asm_symbol, flags)`: copy name to PAD,
   `comp_xt = comp_helper(asm_symbol) or 0`, push `(pad, len, xt, comp_xt, flags)`,
   `call publish_primitive` ( `c-addr u xt comp-xt flags --` ).
5. Read `latest()`, write the xt back-offset cell at `xt-8`.
6. Finalize search order: `CURRENT=forth_wid`, `ORDER_COUNT=3`,
   `CONTEXT=[private,tools,forth]`.

`publish_primitive` fuses `(create) ; (set-xt) ; (set-comp) ; (set-flags)`:
`create` lays the header (defaults `dh_ct=compile_word`, `dh_comp=compile_comma`,
`dh_tfa=0`) + overlay node; then xtptr←xt, optional comp←comp_xt, and `flags!=0` ⇒
`dh_ct=execute`. The PRIMITIVES table (names, asm syms, immediate flag, **order**)
ports verbatim — order is search order, so it's load-bearing.

## 6. The compile-time contract (what drives inlining & typing)

| field | read by | drives |
|---|---|---|
| `dh_ct` | interp `.found_word` | run-immediate (`==execute`) vs compile |
| `dh_xtptr` | dispatcher; WF66 finalize | code addr / typing key / rewrite target |
| `dh_comp` | `compile_word`→`to_comp` | inline-body vs fold vs emit-a-call |
| `dh_ofa` | `inline_comma_word` | # body bytes to copy when inlining (0 → call) |
| `dh_tfa` | `>body`/forget/decompile | colon/create/ivar typing (not the emitter) |

**Reusing the WF66 front-end unchanged (key insight):** the token-IR recorder
(`rt_ir_word`) types a word by matching its **resolved xt** against a boot table of
well-known primitive xts in the user area (`USER_WF66_VOC_*`, `_SEMI/_LBRACE/_TO`).
It does **not** read `dh_ct/dh_comp/dh_stk/dh_tfa`. So the arch-neutral
typing/fold/inline-splice/settle-call/taint logic ports **as-is** provided MF66:
(a) keeps the header offsets + `dh_ct==execute` immediacy, (b) populates the same
`USER_WF66_VOC_*` slots from the AArch64 xts of the same primitive names, and
(c) keeps the `rt_ir_begin`/`rt_ir_word`/`rt_ir_lit`/`rt_ir_finalize` hook sites
(retargeting only the call ABI to AAPCS64). Everything under `lower()` + the
deferred-assembly passes is x86 codegen = the Phase-5 optimizer back-end.

## 7. AArch64 retargets (the only non-identical parts)

1. **CREATE stub — DECISION: store the body address in a leading data cell, not
   baked in the instruction stream.** x86 bakes the body as `mov rax,imm64` and
   `>body` reads `[xt+6]`; on AArch64 `movz/movk` splits the imm across four
   instructions, unrecoverable at a fixed byte offset. So MF66's CREATE word lays:
   `[ body-addr cell | str x0,[DSP,#-8]! ; ldr x0,[pc-relative body cell or via the
   cell] ; sub DSP ; ret ]` — i.e. the executable stub loads the body address from
   the adjacent cell. **`>body` reads that cell (a fixed offset from xt), not
   decoded immediates.** Update `to_body`, `forget` (VAR_HERE/HEAPPTR reclaim),
   `inline_var_comp`, and `create_stub_size` together. (Recommendation (b) from the
   review — cleaner than reconstructing the imm from movz/movk.)
2. **`compile_comma`** emits `call rel32` (±2 GB). AArch64 `bl` reaches only
   ±128 MB, so compile a reference as `bl` when in range, else a veneer
   (`movz/movk x16,xt ; blr x16`). The "dict within ±1.75 GB" assumption is void.
3. **Tail-call patch** `call→jmp` (`E8→E9`) becomes `bl→b` (same imm26 layout — a
   clean 32-bit word patch).
4. **Inline-leaf detection** must decode 32-bit AArch64 words (byte-scanning for
   `E8/E9` is unsafe on fixed-width ISA).
5. **Self-modifying code** (the back-ref write, `inline_*`/`fold_*` emitters, the
   finalize rewrite-over-eager-body) needs, on Apple Silicon: `MAP_JIT` +
   `pthread_jit_write_protect_np` W^X toggling around writes, I-cache maintenance
   (`sys_icache_invalidate` / `dc cvau; ic ivau; isb`), and **16 KB** page math
   (not 4 KB). x86 needs none of these — `MacJit` already handles them for loads.
6. **Position-independent bodies:** WF66's finalize rewinds HERE and copies the
   optimized body over the eager one, so the emitted body must be copy-anywhere
   (no PC-relative code refs). On AArch64 use `movz/movk x16,imm64; blr x16` for
   `Call(xt)`/abs loads rather than ADRP/literal-pool. (Phase-5 concern.)
7. **Hash** uses 32-bit `mul w` (§3). **AAPCS64 caller-saved audit:** WF66's
   `search_header_in_order` spills its loop counter because x86 helpers clobber
   caller-saved regs; on AArch64 hold counter/limit in callee-saved x19–x28 across
   the inner call (or re-spill) — same hazard class, different register split.

## 7a. Memory placement & addressing (AArch64 vs x86 rip-relative)

x86 WF66 keeps the dict heap within ±1.75 GB of code so `call rel32` + rip-relative
`lea` work. AArch64 has no rip-relative *data* load, so the strategy differs:

- **One ±128 MB window for all mutually-calling code.** Compiled colon bodies `bl`
  primitives and each other (`bl` = ±128 MB). So the **dict code/header heap (where
  colon bodies are emitted) must be allocated within ±128 MB of `MacJit`'s primitive
  arena** — then every call is a 1-instruction `bl`; out-of-range falls back to a
  `compile,` veneer (`movz/movk x16; blr x16`). 128 MB easily holds the Forth code.
- **Hot per-task data uses base registers, not PC-relative.** Every user variable
  is `ldr x,[UP,#off]` (one instruction; the 12-bit scaled offset covers the whole
  0…32 KB user area, incl. `VAR_LIMIT`=0x1828). `DSP`/`RP`/`LP` likewise. This is the
  "data base pointer" — better than rip-relative, and what WF66 already does (RBX=UP).
- **Compile-time-known addresses → `adrp+add`** (2 insns, ±4 GB, loader-resolved via
  `@PAGE`/`@PAGEOFF`): the AArch64 analogue of rip-relative, for fixed kernel symbols
  (`compile_word`/`execute`) and folded variable PFAs. Beats `movz/movk` (4 insns)
  when the target is within ±4 GB — so keep the var-data region within ±4 GB of code.
- **Variable/CREATE bodies** are reached via the address baked in the stub (loaded
  once, §7.1), then `@`/`!` operate on a stack address — no PC-relative needed.
- **No dedicated data-base register required** for the dictionary/interpreter (UP
  covers hot vars; a dedicated base only reaches ±32 KB, too small for a growing
  region). x25/x26/x27 remain free if a hot table later wants one.

Net allocation rule: place `{primitive arena, dict code/header heap, var-data
region}` in one ±128 MB span (calls = `bl`; data addresses = `adrp+add`).

## 8. Port phasing

1. **Layout** — add the full dict/overlay/user-var block to `kernel/macros.masm`
   (header `dh_*`, overlay `dn_*`, `wl_*`, `tfa_*`, the dict user vars), byte-identical.
2. **Find/hash** — `init_dictionary_overlay`, `publish_header_overlay` (FNV-1a),
   `search_header_in_wordlist`/`_in_order`, `find_name`. Testable: build a couple
   headers, `find` them.
3. **Construction** — `create`, `publish_primitive`, `set_xt`/`set_comp`/`set_flags`,
   the navigation words (`>ct`/`>comp`/`>name`/`name>interpret`/`name>compile`/`tfa@`),
   `wordlist`/search-order words. (`forget_last`, the CREATE stub, and `>body` come
   with control-flow/CREATE — defer the stub-bearing parts until needed.)
4. **Rust bootstrap** — `Mf66Session`: the §5 sequence; a `PRIMITIVES` table
   (port WF66's verbatim, mapped to MF66 asm symbols), `publish_primitive` driver,
   the xt back-ref writer (with W^X + icache).
5. **Verify** — `find` resolves every bootstrapped primitive; then this unblocks
   the **interpreter** (parse → find → execute/compile) and the eval corpus.

The compile emitters (`compile_comma`/`inline_*`/`fold_*`) and the WF66 front-end
typing land with the interpreter + optimizer phases; this doc fixes the data
layout + bootstrap they all depend on.
