// SPDX-License-Identifier: GPL-2.0-only
//! mica — the shared predicate-language core for flake and borax
//!
//! mica is the small library both build tools link against: the single
//! predicate grammar they must agree on. Like `libalpm` under pacman and
//! makepkg, it is one compilation unit (`libmica.rlib`) shared by two
//! independent front-end binaries — borax (the configuration resolver) and
//! flake (the build engine, which also drives borax as a subprocess). Keep
//! it dependency-free: no `std`-external crates, and nothing tool-specific.
//!
//! It carries the value model, the tokenizer, the `Expr` AST, the
//! recursive-descent parser, version-order string comparison, and an
//! evaluator over a `FnMut(&str) -> Option<Value>` lookup. Errors are plain
//! `String`; borax's `expr.rs` wraps them into its `Diagnostic` type and
//! layers guard-chain (`DefaultSpec`) parsing on top, while flake reads only
//! boolean predicates.
//!
//! Grammar (precedence low → high): `||`, `&&`, `!`, comparison, primary.
//! Atoms are option/key names; a bare atom is a boolean test (`value ==
//! "true"`). Comparisons are `=` (alias `==`), `!=`, `>=`, `<` (with
//! version ordering on strings), and `contains` (comma-list membership).
//!
//! The internals live in sibling modules; this root curates the public API —
//! the surface `configres.rs` (flake) and `expr.rs` (borax) build against —
//! via `pub use`, so the module boundaries can move without touching callers.

mod ast;
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
