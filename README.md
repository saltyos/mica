# mica

`mica` is the configuration language of the SaltyOS build system: one
compilation unit (`libmica.rlib`) used by the **buildutil** engine, which
resolves configuration in process at the start of evaluation, and by the
`buildutil config` commands (menuconfig, explain, docs and the generated
configurations), which drive the same modules, so both produce one set of
values from one set of inputs.

## Modules

- `ast` — the value model (`Value`) and expression AST (`Expr`), with source rendering.
- `lexer` — the tokenizer (`tokenize`).
- `parser` — the recursive-descent parser (`parse_expr`).
- `eval` — the evaluator over a `FnMut(&str) -> Option<Value>` lookup.
- `vercmp` — version-order string comparison.
- `config` — the option graph and its structural validation (`graph`), the
  persistent override file and override layers, resolution (`resolve`) and
  the emitted artifacts (`emit`). `resolve_layers` resolves a graph under the
  persistent file, then each layer in order: an invocation's `-D` overrides,
  then a configuration variant's declaration.

`lib.rs` curates the public API via `pub use`; consumers use `mica::…`.

## Grammar

Precedence low → high: `||`, `&&`, `!`, comparison, primary. Atoms are
option/key names (a bare atom tests `value == "true"`). Comparisons: `=`
(alias `==`), `!=`, `>=`, `<` (version ordering on strings), and `contains`
(comma-list membership).

Dependency-free: no `std`-external crates. Each file's
`SPDX-License-Identifier` line states its license; see `LICENSE.md`.
