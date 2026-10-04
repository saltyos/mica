// SPDX-License-Identifier: GPL-2.0-only
//! mica — the configuration language of buildutil
//!
//! mica is the library the build engine and its configuration commands link
//! against: the predicate grammar and the configuration language they must
//! agree on. It is one compilation unit (`libmica.rlib`) used by the
//! buildutil engine, which resolves configuration in process at the start
//! of evaluation, and by the `buildutil config` commands (menuconfig,
//! explain, docs, generated configurations). Keep it dependency-free: no
//! `std`-external crates, and nothing tool-specific.
//!
//! The predicate core carries the value model, the tokenizer, the `Expr`
//! AST, the recursive-descent parser, version-order string comparison, and
//! an evaluator over a `FnMut(&str) -> Option<Value>` lookup; errors are
//! plain `String`. The `config` module holds the option graph, its
//! resolution, override reading and layering, and the emitted artifacts.
//!
//! Grammar (precedence low → high): `||`, `&&`, `!`, comparison, primary.
//! Atoms are option/key names; a bare atom is a boolean test (`value ==
//! "true"`). Comparisons are `=` (alias `==`), `!=`, `>=`, `<` (with
//! version ordering on strings), and `contains` (comma-list membership of a
//! string or integer literal).
//!
//! This root curates the public API via `pub use`, so the module boundaries
//! can move without touching callers.

mod ast;
pub mod config;
mod eval;
mod lexer;
mod parser;
mod vercmp;

#[cfg(test)]
mod tests;

pub use ast::{CmpOp, Expr, Operand, Value};
pub use eval::eval;
pub use lexer::{Token, tokenize};
pub use parser::{Parser, parse_expr};
pub use vercmp::vercmp;
