// SPDX-License-Identifier: GPL-2.0-only
//! mica — the value model and the `Expr` AST, with source-form rendering.

/// The minimal value an operand resolves to. borax maps its typed option
/// values onto this; flake maps every configuration value as a string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    Bool(bool),
    Str(String),
    Int(i64),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CmpOp {
    Eq,
    Ne,
    Ge,
    Lt,
    /// Comma-list membership: the left value is split on `,`, items are
    /// trimmed, and the right literal must match one exactly.
    Contains,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Operand {
    Name(String),
    Str(String),
    Int(i64),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Expr {
    Atom(String),
    Not(Box<Expr>),
    And(Box<Expr>, Box<Expr>),
    Or(Box<Expr>, Box<Expr>),
    Cmp(CmpOp, Operand, Operand),
}

impl Expr {
    /// Collect every option/key name the expression references, in
    /// left-to-right order (duplicates kept). Used to record a complete,
    /// short-circuit-independent projection of the keys an expression reads.
    pub fn atoms(&self, out: &mut Vec<String>) {
        match self {
            Expr::Atom(name) => out.push(name.clone()),
            Expr::Not(inner) => inner.atoms(out),
            Expr::And(a, b) | Expr::Or(a, b) => {
                a.atoms(out);
                b.atoms(out);
            }
            Expr::Cmp(_, a, b) => {
                for op in [a, b] {
                    if let Operand::Name(name) = op {
                        out.push(name.clone());
                    }
                }
            }
        }
    }

    /// Pretty-print the expression back to source form (for `borax explain`
    /// / `borax docs`); binary sub-expressions are parenthesized to preserve
    /// precedence.
    pub fn render(&self) -> String {
        match self {
            Expr::Atom(n) => n.clone(),
            Expr::Not(e) => format!("!{}", e.render_paren()),
            Expr::And(a, b) => format!("{} && {}", a.render_paren(), b.render_paren()),
            Expr::Or(a, b) => format!("{} || {}", a.render_paren(), b.render_paren()),
            Expr::Cmp(op, a, b) => format!("{} {} {}", a.render(), op.render(), b.render()),
        }
    }

    fn render_paren(&self) -> String {
        match self {
            Expr::And(..) | Expr::Or(..) => format!("({})", self.render()),
            _ => self.render(),
        }
    }
}

impl CmpOp {
    fn render(self) -> &'static str {
        match self {
            CmpOp::Eq => "==",
            CmpOp::Ne => "!=",
            CmpOp::Ge => ">=",
            CmpOp::Lt => "<",
            CmpOp::Contains => "contains",
        }
    }
}

impl Operand {
    fn render(&self) -> String {
        match self {
            Operand::Name(n) => n.clone(),
            Operand::Str(s) => format!("\"{}\"", s),
            Operand::Int(n) => n.to_string(),
        }
    }
}
