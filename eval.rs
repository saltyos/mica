// SPDX-License-Identifier: GPL-2.0-only
//! mica — the evaluator over a `FnMut(&str) -> Option<Value>` lookup.

use crate::ast::{CmpOp, Expr, Operand, Value};
use crate::vercmp::vercmp;
use std::cmp::Ordering;

fn lookup_req(name: &str, look: &mut dyn FnMut(&str) -> Option<Value>) -> Result<Value, String> {
    look(name).ok_or_else(|| format!("unknown name `{}`", name))
}

fn operand_value(
    op: &Operand,
    look: &mut dyn FnMut(&str) -> Option<Value>,
) -> Result<Value, String> {
    match op {
        Operand::Name(name) => lookup_req(name, look),
        Operand::Str(s) => Ok(Value::Str(s.clone())),
        Operand::Int(n) => Ok(Value::Int(*n)),
    }
}

/// Evaluate a boolean predicate. `look` resolves a name to its current
/// value; a bare atom is true when that value is a `true` bool or the
/// string `"true"`.
pub fn eval(expr: &Expr, look: &mut dyn FnMut(&str) -> Option<Value>) -> Result<bool, String> {
    match expr {
        Expr::Atom(name) => match lookup_req(name, look)? {
            Value::Bool(b) => Ok(b),
            Value::Str(s) => Ok(s == "true"),
            Value::Int(_) => Err(format!("`{}` is an integer, not a boolean atom", name)),
        },
        Expr::Not(inner) => Ok(!eval(inner, look)?),
        Expr::And(a, b) => Ok(eval(a, look)? && eval(b, look)?),
        Expr::Or(a, b) => Ok(eval(a, look)? || eval(b, look)?),
        Expr::Cmp(op, lhs, rhs) => {
            let l = operand_value(lhs, look)?;
            let r = operand_value(rhs, look)?;
            cmp_values(*op, &l, &r)
        }
    }
}

fn cmp_values(op: CmpOp, l: &Value, r: &Value) -> Result<bool, String> {
    if op == CmpOp::Contains {
        return match (l, r) {
            (Value::Str(hay), Value::Str(needle)) => {
                Ok(hay.split(',').map(str::trim).any(|item| item == needle))
            }
            _ => Err(format!(
                "`contains` needs string operands: {:?} vs {:?}",
                l, r
            )),
        };
    }
    match (l, r) {
        (Value::Int(a), Value::Int(b)) => Ok(match op {
            CmpOp::Eq => a == b,
            CmpOp::Ne => a != b,
            CmpOp::Ge => a >= b,
            CmpOp::Lt => a < b,
            CmpOp::Contains => unreachable!(),
        }),
        (Value::Str(a), Value::Str(b)) => Ok(match op {
            CmpOp::Eq => a == b,
            CmpOp::Ne => a != b,
            CmpOp::Ge => vercmp(a, b) != Ordering::Less,
            CmpOp::Lt => vercmp(a, b) == Ordering::Less,
            CmpOp::Contains => unreachable!(),
        }),
        (Value::Bool(a), Value::Bool(b)) => match op {
            CmpOp::Eq => Ok(a == b),
            CmpOp::Ne => Ok(a != b),
            _ => Err("booleans support only `=` / `!=`".to_string()),
        },
        _ => Err(format!("type mismatch in comparison: {:?} vs {:?}", l, r)),
    }
}
