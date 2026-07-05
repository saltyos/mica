// SPDX-License-Identifier: GPL-2.0-only
//! mica — the tokenizer.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Token {
    Ident(String),
    Str(String),
    Int(i64),
    Not,
    AndAnd,
    OrOr,
    LParen,
    RParen,
    Eq,
    Ne,
    Ge,
    Lt,
    Contains,
    If,
    Else,
}

pub(crate) fn err(src: &str, msg: impl Into<String>) -> String {
    format!("{} in expression `{}`", msg.into(), src)
}

pub fn tokenize(src: &str) -> Result<Vec<Token>, String> {
    let mut toks = Vec::new();
    let bytes = src.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        match b {
            b' ' | b'\t' => i += 1,
            b'(' => {
                toks.push(Token::LParen);
                i += 1;
            }
            b')' => {
                toks.push(Token::RParen);
                i += 1;
            }
            b'&' => {
                if bytes.get(i + 1) == Some(&b'&') {
                    toks.push(Token::AndAnd);
                    i += 2;
                } else {
                    return Err(err(src, "expected `&&`"));
                }
            }
            b'|' => {
                if bytes.get(i + 1) == Some(&b'|') {
                    toks.push(Token::OrOr);
                    i += 2;
                } else {
                    return Err(err(src, "expected `||`"));
                }
            }
            b'!' => {
                if bytes.get(i + 1) == Some(&b'=') {
                    toks.push(Token::Ne);
                    i += 2;
                } else {
                    toks.push(Token::Not);
                    i += 1;
                }
            }
            b'=' => {
                if bytes.get(i + 1) == Some(&b'=') {
                    toks.push(Token::Eq);
                    i += 2;
                } else {
                    toks.push(Token::Eq);
                    i += 1;
                }
            }
            b'>' => {
                if bytes.get(i + 1) == Some(&b'=') {
                    toks.push(Token::Ge);
                    i += 2;
                } else {
                    return Err(err(src, "`>` is not supported; use `>=` or swap to `<`"));
                }
            }
            b'<' => {
                if bytes.get(i + 1) == Some(&b'=') {
                    return Err(err(src, "`<=` is not supported; use `<` or swap to `>=`"));
                }
                toks.push(Token::Lt);
                i += 1;
            }
            b'"' => {
                let mut out = String::new();
                let mut j = i + 1;
                let mut closed = false;
                while j < bytes.len() {
                    match bytes[j] {
                        b'"' => {
                            closed = true;
                            break;
                        }
                        b'\\' => {
                            j += 1;
                            match bytes.get(j) {
                                Some(b'"') => out.push('"'),
                                Some(b'\\') => out.push('\\'),
                                _ => return Err(err(src, "unsupported escape in string")),
                            }
                        }
                        c => out.push(c as char),
                    }
                    j += 1;
                }
                if !closed {
                    return Err(err(src, "unterminated string literal"));
                }
                toks.push(Token::Str(out));
                i = j + 1;
            }
            _ if b.is_ascii_digit()
                || (b == b'-' && bytes.get(i + 1).is_some_and(|c| c.is_ascii_digit())) =>
            {
                let start = i;
                i += 1;
                while i < bytes.len() && bytes[i].is_ascii_digit() {
                    i += 1;
                }
                let word = &src[start..i];
                let n: i64 = word
                    .parse()
                    .map_err(|_| err(src, format!("integer out of range: `{}`", word)))?;
                toks.push(Token::Int(n));
            }
            _ if b.is_ascii_alphabetic() || b == b'_' => {
                let start = i;
                while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                    i += 1;
                }
                match &src[start..i] {
                    "if" => toks.push(Token::If),
                    "else" => toks.push(Token::Else),
                    "contains" => toks.push(Token::Contains),
                    name => toks.push(Token::Ident(name.to_string())),
                }
            }
            _ => {
                return Err(err(src, format!("unexpected character `{}`", b as char)));
            }
        }
    }
    Ok(toks)
}
