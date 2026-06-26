# MF66 memory map

MF66 runs in three independent allocations:

1. **The data region** — one 8 MiB `alloc_zeroed` block holding all the Forth
   stacks, the user area, the dictionary, and data space. Addresses below are
   *offsets* into this block (the live base is `region`; `user_base = region +
   0x100000`).
2. **The kernel JIT region** — a single `MAP_JIT` mapping (`MacJit`) holding the
   assembled kernel + primitives.
3. **The colon-body arena** — a second `MAP_JIT` mapping (`CodeArena`) holding
   JIT-compiled colon-word / method bodies and the bodies of `constant` /
   `variable` / `create` words.

Calls between the two code regions go through a uniform veneer (`movz/movk x16,
xt; blr x16`) so distance never matters. W^X is handled per-thread with
`pthread_jit_write_protect_np` + `sys_icache_invalidate`.

## Register ABI (AArch64 / AAPCS64)

| Reg | Role | Notes |
|-----|------|-------|
| `x0`  | **TOS** — cached top of data stack | every primitive reads/writes it |
| `x19` | **DSP** — data-stack pointer (points at NOS) | grows down; push = `str x0,[DSP,#-8]!` |
| `x20` | **UP** — user-area base | callee-saved; survives host callouts |
| `x21` | **LP** — locals-frame pointer | `{: :}` frames |
| `x22` | **FSP** — float-stack pointer | parked in `user_FSP` across boundaries |
| `x28` | **RP** — return stack | plain 8-byte full-descending stack, **not** `sp` |
| `d8`  | **FTOS** — cached float top | parked in `user_FTOS_SAVE` |
| `x16`/`x17` | veneer scratch | forbidden in register pools |
| `x18` | **forbidden** (Darwin-reserved) | |
| `x9`–`x15` | caller-saved scratch | the optimizer's register window |
| `sp` | the C / OS stack | only `forth_main`'s 160-byte frame touches it; always 16-aligned |

STC: every word is reached by `blr`/veneer and ends in `ret`; a colon body's
`nest` is `str x30,[RP,#-8]!` and `unnest` is `ldr x30,[RP],#8; ret`. Because
`bl`/`blr` write `x30` (not memory), the return stack is decoupled from `sp` —
which is why `RP` is a dedicated register and `>r`/`r>` are trivial.

## The 8 MiB data region

```
0x000000 ┌─────────────────────────────┐
         │ DATA STACK   (512 KiB)       │  DSP=x19 grows DOWN from 0x080000
0x080000 ├─────────────────────────────┤
         │ RETURN STACK (512 KiB)       │  RP=x28  grows DOWN from 0x100000
0x100000 ├─────────────────────────────┤
         │ USER AREA    (512 KiB)       │  UP=x20 base; offset table below
0x180000 ├─────────────────────────────┤
         │ LOCALS STACK (512 KiB)       │  LP=x21 grows DOWN from 0x200000
0x200000 ├─────────────────────────────┤
         │ DICTIONARY   (4 MiB)         │  headers/HERE grow UP from 0x200000;
         │                              │  index/overlay arena grows DOWN from 0x600000
0x600000 ├─────────────────────────────┤
         │ DATA SPACE   (2 MiB)         │  VAR_HERE grows UP from 0x600000;
0x800000 └─────────────────────────────┘  VAR_LIMIT = 0x800000
```

Two regions deliberately grow toward each other and meet in the middle:
- **Dictionary** (`0x200000–0x600000`): the RW header/code heap grows up
  (`HERE`), while the hash/wordlist overlay arena grows down (`INDEX_HERE`).
- **Data space** (`0x600000–0x800000`): variable bodies, `create` data fields,
  object instances, and class structs are bump-allocated up from `VAR_HERE`.

`constant`/`variable`/`create`/colon/method *code* does **not** live here — it
lives in the `CodeArena` JIT mapping; only their *data* (the variable cell, the
class struct, the instance) lives in data space.

## User-area offset table (base = `0x100000`, UP = x20)

| Offset | Name | Meaning |
|--------|------|---------|
| `0x00` | BASE | numeric base (10 default) |
| `0x08` | STATE | 0 interpret / 1 compile |
| `0x10` | LATEST | head of the dictionary chain |
| `0x18` | HERE | dict-heap bump pointer |
| `0x20` | DICT_END | dict-heap high limit |
| `0x28`–`0x40` | SOURCE_ID / SOURCE_ADDR / SOURCE_LEN / >IN | input source + parse offset |
| `0x50` | BYE_REQ | cooperative `bye` flag |
| `0x58` | HOST_RSP | host `sp` stashed by `forth_main` |
| `0x60` | DSP_SAVE | logical DSP published on exit |
| `0x68` | SP0 | initial DSP (= dstack_top); read by `depth` |
| `0x70` | RSP_CURRENT | Forth return-stack ptr across `forth_main` calls |
| `0x78` | LATESTXT | xt of the most recent definition |
| `0x80` | HANDLER | catch/throw handler-frame chain head (on RP) |
| `0x88` | THROW_CODE | uncaught THROW code |
| `0x100` | PAD | 256 B scratch |
| `0x200` | SOURCE_BUF | 4 KiB line buffer |
| `0x1210` | FP0 | empty float-stack pointer |
| `0x1218` | FSP | current float-stack pointer |
| `0x1228` | FTOS_SAVE | `d8` parked across `forth_main` |
| `0x1300` | FP_STACK | 256 B float stack (FSP grows down from 0x1400) |
| `0x1400` | WORD_BUF | 256 B counted-string buffer |
| `0x1500`–`0x17D0` | CURRENT / FORTH_WID / search order / wordlists | vocabulary state |
| `0x15B0` | LP0 | initial locals-stack pointer |
| `0x1820` | VAR_HERE | data-space bump pointer |
| `0x1828` | VAR_LIMIT | data-space high limit (= 0x800000) |
| `0x1830` | SELF | OOP current receiver (object base addr) |

## Object / class layout (data space)

```
instance:  [obj+0]  = class struct addr
           [obj+8]  = ivar0   [obj+16] = ivar1   …      (offsets from ClassInfo)

class:     [class+0]  = super struct addr (0 for the root `object`)
           [class+8]  = reserved
           [class+16] = vtable[256]   selector id k → method xt at class+16+k*8
                        (unused slots = the (dnu) xt → THROW -2058)
```

`self` (in `user_SELF`) is the receiver; `(send)`/`(send-xt)` set it around the
method call and restore it afterwards, parking both the previous self and the
dispatcher's own return address on **RP** so a `throw` inside a method unwinds
cleanly (`RP := HANDLER`) without leaking the C stack.
