# mica

`mica` is the shared predicate-language core for the SaltyOS build system: one
compilation unit (`libmica.rlib`) linked by two independent host build tools —
**borax** (the configuration resolver) and **flake** (the build engine) — the
single predicate grammar they must agree on. The libalpm-to-pacman
relationship: a shared core library beneath two front-end binaries.

## Modules

- `ast` — the value model (`Value`) and expression AST (`Expr`), with source rendering.
- `lexer` — the tokenizer (`tokenize`).
- `parser` — the recursive-descent parser (`parse_expr`).
- `eval` — the evaluator over a `FnMut(&str) -> Option<Value>` lookup.
- `vercmp` — version-order string comparison.

`lib.rs` curates the public API via `pub use`; consumers use `mica::…`.

## Grammar

Precedence low → high: `||`, `&&`, `!`, comparison, primary. Atoms are
option/key names (a bare atom tests `value == "true"`). Comparisons: `=`
(alias `==`), `!=`, `>=`, `<` (version ordering on strings), and `contains`
(comma-list membership).

Dependency-free: no `std`-external crates. Licensed GPL-2.0-only.
