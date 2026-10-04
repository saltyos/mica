// SPDX-License-Identifier: LGPL-2.1-or-later
//! mica — guard-default chains and the diagnostic-aware predicate surface
//!
//! The option graph carries typed literals in its guarded defaults, so the
//! chain parser lives beside the graph rather than in the predicate core.
//! This module wraps core parse errors into `Diagnostic` and bridges the
//! closure-based evaluator onto the resolver's typed `EvalCtx`; the engine's
//! `when` predicates use the core parser directly.

use crate::config::diag::{Code, Diagnostic};
use crate::config::toml::Value;
use crate::Token;

pub use crate::Expr;

/// A resolved operand value. `buildutil config` maps its typed option values onto the
/// shared `mica` value model; the alias keeps every call site readable.
pub type CmpVal = crate::Value;

/// A guard-default chain: arms are tried in order; the first arm whose guard
/// holds (or has no guard) supplies the value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DefaultSpec {
    pub arms: Vec<(Value, Option<Expr>)>,
}

fn parse_err(src: &str, msg: String) -> Diagnostic {
    let _ = src;
    Diagnostic::new(Code::EParse, msg)
}

/// Parse a boolean predicate (`depends_on`, `visible_when`, guard bodies).
pub fn parse_expr(src: &str) -> Result<Expr, Diagnostic> {
    crate::parse_expr(src).map_err(|m| parse_err(src, m))
}

/// Parse a guard-default chain: `VALUE [if EXPR [else VALUE if EXPR ...]]`.
pub fn parse_default(src: &str) -> Result<DefaultSpec, Diagnostic> {
    let toks = crate::tokenize(src).map_err(|m| parse_err(src, m))?;
    let mut p = crate::Parser::new(&toks, src);
    let mut arms = Vec::new();
    loop {
        let value = match p.next() {
            Some(Token::Str(s)) => Value::Str(s),
            Some(Token::Int(n)) => Value::Int(n),
            Some(Token::Ident(w)) if w == "true" => Value::Bool(true),
            Some(Token::Ident(w)) if w == "false" => Value::Bool(false),
            _ => {
                return Err(parse_err(
                    src,
                    format!("expected a literal default value in `{}`", src),
                ));
            }
        };
        match p.peek() {
            None => {
                arms.push((value, None));
                break;
            }
            Some(Token::If) => {
                p.next();
                let guard = p.parse_or().map_err(|m| parse_err(src, m))?;
                arms.push((value, Some(guard)));
                match p.next() {
                    None => break,
                    Some(Token::Else) => continue,
                    _ => {
                        return Err(parse_err(
                            src,
                            format!("expected `else` or end of default in `{}`", src),
                        ));
                    }
                }
            }
            _ => {
                return Err(parse_err(
                    src,
                    format!("expected `if` or end of default in `{}`", src),
                ));
            }
        }
    }
    Ok(DefaultSpec { arms })
}

impl DefaultSpec {
    pub fn literal(value: Value) -> Self {
        DefaultSpec {
            arms: vec![(value, None)],
        }
    }

    pub fn atoms(&self, out: &mut Vec<String>) {
        for (_, guard) in &self.arms {
            if let Some(g) = guard {
                g.atoms(out);
            }
        }
    }

    /// Render the guard chain back to source form for `buildutil config explain` / docs.
    pub fn render(&self) -> String {
        let mut parts = Vec::new();
        for (value, guard) in &self.arms {
            let v = match value {
                Value::Bool(b) => b.to_string(),
                Value::Int(n) => n.to_string(),
                Value::Str(s) => format!("\"{}\"", s),
                other => format!("{:?}", other),
            };
            match guard {
                Some(g) => parts.push(format!("{} if {}", v, g.render())),
                None => parts.push(v),
            }
        }
        parts.join(" else ")
    }
}

/// Resolution callback: maps an option/choice name to its current value.
pub trait EvalCtx {
    fn lookup(&mut self, name: &str) -> Result<CmpVal, Diagnostic>;
}

/// Evaluate a predicate against `ctx`. Bridges the closure-based core
/// evaluator onto the typed `EvalCtx`: a lookup diagnostic (unknown ref,
/// type mismatch) is captured and returned unchanged; a core-only error
/// (bad atom type, comparison mismatch) surfaces as `E_TYPE`.
pub fn eval(expr: &Expr, ctx: &mut dyn EvalCtx) -> Result<bool, Diagnostic> {
    let mut captured: Option<Diagnostic> = None;
    let result = {
        let mut look = |name: &str| -> Option<CmpVal> {
            match ctx.lookup(name) {
                Ok(v) => Some(v),
                Err(d) => {
                    captured = Some(d);
                    None
                }
            }
        };
        crate::eval(expr, &mut look)
    };
    match result {
        Ok(b) => Ok(b),
        Err(msg) => Err(captured.unwrap_or_else(|| Diagnostic::new(Code::EType, msg))),
    }
}

/// Evaluate a guard-default chain; returns the first arm whose guard holds.
pub fn eval_default(
    spec: &DefaultSpec,
    ctx: &mut dyn EvalCtx,
) -> Result<Option<Value>, Diagnostic> {
    for (value, guard) in &spec.arms {
        match guard {
            None => return Ok(Some(value.clone())),
            Some(g) => {
                if eval(g, ctx)? {
                    return Ok(Some(value.clone()));
                }
            }
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    struct MapCtx(BTreeMap<String, CmpVal>);

    impl EvalCtx for MapCtx {
        fn lookup(&mut self, name: &str) -> Result<CmpVal, Diagnostic> {
            self.0
                .get(name)
                .cloned()
                .ok_or_else(|| Diagnostic::new(Code::EUnknownRef, format!("unknown `{}`", name)))
        }
    }

    fn ctx(pairs: &[(&str, CmpVal)]) -> MapCtx {
        MapCtx(
            pairs
                .iter()
                .map(|(k, v)| (k.to_string(), v.clone()))
                .collect(),
        )
    }

    #[test]
    fn precedence_and_parens() {
        let mut c = ctx(&[
            ("A", CmpVal::Bool(true)),
            ("B", CmpVal::Bool(false)),
            ("C", CmpVal::Bool(true)),
        ]);
        let e = parse_expr("!B && A || B").unwrap();
        assert!(eval(&e, &mut c).unwrap());
        let e = parse_expr("!(B || C) && A").unwrap();
        assert!(!eval(&e, &mut c).unwrap());
    }

    #[test]
    fn comparisons() {
        let mut c = ctx(&[
            ("LEVEL", CmpVal::Str("info".into())),
            ("NCURSES", CmpVal::Str("6.5".into())),
            ("N", CmpVal::Int(16)),
        ]);
        assert!(eval(&parse_expr("LEVEL = \"info\"").unwrap(), &mut c).unwrap());
        assert!(eval(&parse_expr("LEVEL != \"debug\"").unwrap(), &mut c).unwrap());
        assert!(eval(&parse_expr("NCURSES >= \"6.4\"").unwrap(), &mut c).unwrap());
        assert!(!eval(&parse_expr("NCURSES < \"6.5\"").unwrap(), &mut c).unwrap());
        assert!(eval(&parse_expr("N >= 16 && N < 17").unwrap(), &mut c).unwrap());
    }

    #[test]
    fn unknown_ref_preserves_code() {
        let mut c = ctx(&[]);
        let d = eval(&parse_expr("MISSING").unwrap(), &mut c).unwrap_err();
        assert_eq!(d.code, Code::EUnknownRef);
    }

    #[test]
    fn default_chains() {
        let spec = parse_default("true if !DEBUG_SERIAL else false").unwrap();
        assert_eq!(spec.arms.len(), 2);
        let mut c = ctx(&[("DEBUG_SERIAL", CmpVal::Bool(false))]);
        assert_eq!(
            eval_default(&spec, &mut c).unwrap(),
            Some(Value::Bool(true))
        );
        let mut c = ctx(&[("DEBUG_SERIAL", CmpVal::Bool(true))]);
        assert_eq!(
            eval_default(&spec, &mut c).unwrap(),
            Some(Value::Bool(false))
        );

        let spec = parse_default("32 if BIG").unwrap();
        let mut c = ctx(&[("BIG", CmpVal::Bool(false))]);
        assert_eq!(eval_default(&spec, &mut c).unwrap(), None);

        let spec = parse_default("\"a\" if X = 1 else \"b\" if X = 2 else \"c\"").unwrap();
        let mut c = ctx(&[("X", CmpVal::Int(2))]);
        assert_eq!(
            eval_default(&spec, &mut c).unwrap(),
            Some(Value::Str("b".into()))
        );
    }

    #[test]
    fn parse_errors() {
        assert!(parse_expr("A &&").is_err());
        assert!(parse_expr("A > B").is_err());
        assert!(parse_expr("\"lit\"").is_err());
        assert!(parse_expr("A B").is_err());
        assert!(parse_default("if A").is_err());
    }

    #[test]
    fn atom_collection() {
        let e = parse_expr("A && (B || C = \"x\") && D >= 2").unwrap();
        let mut out = Vec::new();
        e.atoms(&mut out);
        assert_eq!(out, vec!["A", "B", "C", "D"]);
    }
}
