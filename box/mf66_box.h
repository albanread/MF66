/* SPDX-License-Identifier: MIT
 * Copyright (c) 2026 Alban Read */

/* mf66_box.h: MF66, the optimizing Forth, as a library for the BOX
 * (box/src/lib.rs; built by box/build.sh into libmf66-box.a).  The host
 * (A7232ToolChain rosgd/modules/forth) fills in the hooks; each one is
 * optional.  One session at a time, used from one thread. */
#ifndef MF66_BOX_H
#define MF66_BOX_H

#include <stddef.h>
#include <stdint.h>

struct mf66_hooks {
    void (*write)(const uint8_t *p, size_t n);          /* output as it is made */
    int64_t (*key)(void);                                /* key: a character, -1 at the end */
    int64_t (*key_ready)(void);                          /* key? */
    int64_t (*read_line)(uint8_t *buf, size_t max);      /* accept: its length, -1 */
    uint8_t *(*region_alloc)(size_t n);                  /* the data region, zeroed */
    void (*region_free)(uint8_t *p, size_t n);
    uint8_t *(*mem_alloc)(size_t n);                     /* allocate free resize */
    void (*mem_free)(uint8_t *p);
    uint8_t *(*mem_resize)(uint8_t *p, size_t n);
    /* a SWI: number, R0..R9 in and out, flags out (N Z C V = bits 3..0);
     * 0 or the error block's address */
    uint64_t (*swi)(uint64_t number, uint64_t regs[10], uint64_t *flags);
    int64_t (*swi_number)(const uint8_t *name, size_t n);  /* -1 if none */
    void (*ms)(uint64_t ms);
    /* files: open modes 1 read, 2 write, 3 update, 4 create; handle or 0 */
    uint64_t (*file_open)(const uint8_t *name, size_t n, uint64_t mode);
    int64_t (*file_close)(uint64_t h);
    int64_t (*file_read)(uint64_t h, uint8_t *buf, size_t n);
    int64_t (*file_write)(uint64_t h, const uint8_t *buf, size_t n);
    int64_t (*file_ptr)(uint64_t h, uint64_t set, int64_t pos);  /* get (set 0) or set */
    int64_t (*file_extent)(uint64_t h);
    int64_t (*file_delete)(const uint8_t *name, size_t n);
    int64_t (*file_rename)(const uint8_t *a, size_t an, const uint8_t *b, size_t bn);
};

void mf66_box_set_hooks(const struct mf66_hooks *h);
int mf66_box_boot(void);        /* a fresh session: 0, or -1 (the reason written) */
void mf66_box_end(void);
int mf66_box_running(void);
/* one REPL line: 0, 1 at bye, -1 an error (written; input state reset) */
int mf66_box_line(const char *p, size_t n);
/* a whole source, errors as name:line: error: ...: 0, 1 at bye, -1 */
int mf66_box_eval(const char *p, size_t n, const char *name, size_t namelen);
int mf66_box_compiling(void);   /* a definition is open */
void mf66_box_recover(void);    /* after a fault escaped from Forth code */

#endif
