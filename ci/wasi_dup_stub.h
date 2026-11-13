// Shared stub for dup() when building tree-sitter 0.20 grammars for wasm32-wasip1.
// tree-sitter's src/tree.c calls dup() unconditionally (debug only); wasi-libc
// does not provide dup(), so we stub it here via -include in CFLAGS.
#pragma once
#ifdef __wasi__
#include <unistd.h>
static inline int dup(int fd) { (void)fd; return -1; }
#endif
