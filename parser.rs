// SPDX-License-Identifier: GPL-2.0-only
//! mica — the recursive-descent parser.
//!
//! Grammar (precedence low → high): `||`, `&&`, `!`, comparison, primary.

use crate::ast::{CmpOp, Expr, Operand};
use crate::lexer::{Token, err, tokenize};

pub struct Parser<'a> {
    toks: &'a [Token],
    pos: usize,
    src: &'a str,
}

impl<'a> Parser<'a> {
    pub fn new(toks: &'a [Token], src: &'a str) -> Self {
        Parser { toks, pos: 0, src }
    }

    pub fn peek(&self) -> Option<&Token> {
        self.toks.get(self.pos)
    }

    pub fn next(&mut self) -> Option<Token> {
        let t = self.toks.get(self.pos).cloned();
        if t.is_some() {
            self.pos += 1;
        }
        t
    }

    pub fn at_end(&self) -> bool {
        self.pos == self.toks.len()
    }

    pub fn parse_or(&mut self) -> Result<Expr, String> {
        let mut lhs = self.parse_and()?;
        while self.peek() == Some(&Token::OrOr) {
            self.next();
            let rhs = self.parse_and()?;
            lhs = Expr::Or(Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    fn parse_and(&mut self) -> Result<Expr, String> {
        let mut lhs = self.parse_unary()?;
        while self.peek() == Some(&Token::AndAnd) {
            self.next();
            let rhs = self.parse_unary()?;
            lhs = Expr::And(Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    fn parse_unary(&mut self) -> Result<Expr, String> {
        if self.peek() == Some(&Token::Not) {
            self.next();
            let inner = self.parse_unary()?;
            return Ok(Expr::Not(Box::new(inner)));
        }
        self.parse_primary()
    }

    fn cmp_op(&mut self) -> Option<CmpOp> {
        let op = match self.peek() {
            Some(Token::Eq) => CmpOp::Eq,
            Some(Token::Ne) => CmpOp::Ne,
            Some(Token::Ge) => CmpOp::Ge,
            Some(Token::Lt) => CmpOp::Lt,
            Some(Token::Contains) => CmpOp::Contains,
            _ => return None,
        };
        self.next();
        Some(op)
    }

    fn parse_operand(&mut self) -> Result<Operand, String> {
        match self.next() {
            Some(Token::Ident(name)) => Ok(Operand::Name(name)),
            Some(Token::Str(s)) => Ok(Operand::Str(s)),
            Some(Token::Int(n)) => Ok(Operand::Int(n)),
            _ => Err(err(self.src, "expected an option name or literal")),
        }
    }

    fn parse_primary(&mut self) -> Result<Expr, String> {
        if self.peek() == Some(&Token::LParen) {
            self.next();
            let inner = self.parse_or()?;
            if self.next() != Some(Token::RParen) {
                return Err(err(self.src, "expected `)`"));
            }
            return Ok(inner);
        }
        let lhs = self.parse_operand()?;
        if let Some(op) = self.cmp_op() {
            let rhs = self.parse_operand()?;
            return Ok(Expr::Cmp(op, lhs, rhs));
        }
        match lhs {
            Operand::Name(name) => Ok(Expr::Atom(name)),
            _ => Err(err(self.src, "a bare literal is not a boolean expression")),
        }
    }
}

pub fn parse_expr(src: &str) -> Result<Expr, String> {
    let toks = tokenize(src)?;
    let mut p = Parser::new(&toks, src);
    let expr = p.parse_or()?;
    if !p.at_end() {
        return Err(err(src, "trailing tokens"));
    }
    Ok(expr)
}
