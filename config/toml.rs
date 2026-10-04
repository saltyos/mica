// SPDX-License-Identifier: LGPL-2.1-or-later
//! mica — strict TOML-subset parser
//!
//! The accepted grammar is a deliberately small subset; anything outside it
//! is E_PARSE. Supported: `#` comments (full-line and trailing), table
//! headers with dotted bare keys (`[option.NET]`,
//! `[choice.console_backend.option.CONSOLE_SERIAL]`), one `key = value`
//! pair per line, values as basic double-quoted strings (escapes: \" \\
//! \n \t), decimal integers, `true`/`false`, and single-line arrays of
//! strings or of integers. Rejected: inline tables, multi-line strings, literal strings,
//! floats, dates, dotted keys outside headers, duplicate tables/keys.

use crate::config::diag::{Code, Diagnostic};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    Str(String),
    Int(i64),
    Bool(bool),
    Array(Vec<String>),
    /// A single-line integer array: the value and default form of an
    /// `int-list` option.
    IntList(Vec<i64>),
}

impl Value {
    pub fn type_name(&self) -> &'static str {
        match self {
            Value::Str(_) => "string",
            Value::Int(_) => "int",
            Value::Bool(_) => "bool",
            Value::Array(_) => "array",
            Value::IntList(_) => "int-list",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Entry {
    pub key: String,
    pub value: Value,
    pub line: u32,
}

#[derive(Debug, Clone)]
pub struct Table {
    /// Dotted header path; the implicit root table has an empty path.
    pub path: Vec<String>,
    pub line: u32,
    pub entries: Vec<Entry>,
}

#[derive(Debug, Clone)]
pub struct Doc {
    pub file: PathBuf,
    /// Tables in file order; index 0 is the implicit root table.
    pub tables: Vec<Table>,
}

fn is_bare_key_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'-'
}

fn err(file: &Path, line: u32, msg: impl Into<String>) -> Diagnostic {
    Diagnostic::new(Code::EParse, msg).at(file, line)
}

/// Parse a basic double-quoted string starting at `s` (which begins with `"`).
/// Returns (content, rest-after-closing-quote).
fn parse_string<'a>(s: &'a str, file: &Path, line: u32) -> Result<(String, &'a str), Diagnostic> {
    debug_assert!(s.starts_with('"'));
    let bytes = s.as_bytes();
    let mut out = String::new();
    let mut i = 1;
    while i < bytes.len() {
        match bytes[i] {
            b'"' => return Ok((out, &s[i + 1..])),
            b'\\' => {
                i += 1;
                match bytes.get(i) {
                    Some(b'"') => out.push('"'),
                    Some(b'\\') => out.push('\\'),
                    Some(b'n') => out.push('\n'),
                    Some(b't') => out.push('\t'),
                    _ => return Err(err(file, line, "unsupported escape in string")),
                }
            }
            _ => {
                // Copy one whole UTF-8 scalar.
                let ch_len = s[i..].chars().next().map(|c| c.len_utf8()).unwrap_or(1);
                out.push_str(&s[i..i + ch_len]);
                i += ch_len - 1;
            }
        }
        i += 1;
    }
    Err(err(file, line, "unterminated string"))
}

/// Parse one value starting at `s`; returns (value, rest).
fn parse_value<'a>(s: &'a str, file: &Path, line: u32) -> Result<(Value, &'a str), Diagnostic> {
    let s = s.trim_start();
    if s.starts_with('"') {
        let (v, rest) = parse_string(s, file, line)?;
        return Ok((Value::Str(v), rest));
    }
    if let Some(rest) = s.strip_prefix('[') {
        let mut strings = Vec::new();
        let mut ints = Vec::new();
        let mut cur = rest.trim_start();
        loop {
            if let Some(after) = cur.strip_prefix(']') {
                // An empty array carries no element type; the graph reads it
                // as either list form.
                if ints.is_empty() {
                    return Ok((Value::Array(strings), after));
                }
                return Ok((Value::IntList(ints), after));
            }
            if cur.starts_with('"') {
                if !ints.is_empty() {
                    return Err(err(file, line, "arrays may not mix strings and integers"));
                }
                let (item, rest2) = parse_string(cur, file, line)?;
                strings.push(item);
                cur = rest2.trim_start();
            } else {
                if !strings.is_empty() {
                    return Err(err(file, line, "arrays may not mix strings and integers"));
                }
                let end = cur
                    .as_bytes()
                    .iter()
                    .position(|&b| !(b.is_ascii_digit() || b == b'-' || b == b'+'))
                    .unwrap_or(cur.len());
                if end == 0 {
                    return Err(err(
                        file,
                        line,
                        "arrays may contain only strings or only integers",
                    ));
                }
                ints.push(parse_int_word(&cur[..end], file, line)?);
                cur = cur[end..].trim_start();
            }
            if let Some(after) = cur.strip_prefix(',') {
                cur = after.trim_start();
            } else if !cur.starts_with(']') {
                return Err(err(file, line, "expected ',' or ']' in array"));
            }
        }
    }
    // Bare word: bool or integer.
    let end = s
        .as_bytes()
        .iter()
        .position(|&b| !(is_bare_key_byte(b) || b == b'+'))
        .unwrap_or(s.len());
    let word = &s[..end];
    let rest = &s[end..];
    match word {
        "true" => return Ok((Value::Bool(true), rest)),
        "false" => return Ok((Value::Bool(false), rest)),
        "" => return Err(err(file, line, "expected a value")),
        _ => {}
    }
    Ok((Value::Int(parse_int_word(word, file, line)?), rest))
}

/// A decimal integer with an optional leading minus sign.
fn parse_int_word(word: &str, file: &Path, line: u32) -> Result<i64, Diagnostic> {
    let (neg, digits) = match word.strip_prefix('-') {
        Some(d) => (true, d),
        None => (false, word),
    };
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return Err(err(file, line, format!("invalid value `{}`", word)));
    }
    let mag: i64 = digits
        .parse()
        .map_err(|_| err(file, line, format!("integer out of range: `{}`", word)))?;
    Ok(if neg { -mag } else { mag })
}

/// The remainder of a line after a parsed construct must be blank or a comment.
fn expect_line_end(rest: &str, file: &Path, line: u32) -> Result<(), Diagnostic> {
    let rest = rest.trim_start();
    if rest.is_empty() || rest.starts_with('#') {
        Ok(())
    } else {
        Err(err(
            file,
            line,
            format!("unexpected trailing content: `{}`", rest),
        ))
    }
}

fn parse_header(s: &str, file: &Path, line: u32) -> Result<Vec<String>, Diagnostic> {
    debug_assert!(s.starts_with('['));
    let Some(close) = s.find(']') else {
        return Err(err(file, line, "unterminated table header"));
    };
    let inner = &s[1..close];
    expect_line_end(&s[close + 1..], file, line)?;
    let mut path = Vec::new();
    for part in inner.split('.') {
        let part = part.trim();
        if part.is_empty() || !part.bytes().all(is_bare_key_byte) {
            return Err(err(
                file,
                line,
                format!("invalid table header `[{}]`", inner),
            ));
        }
        path.push(part.to_string());
    }
    Ok(path)
}

pub fn parse(file: &Path, src: &str) -> Result<Doc, Diagnostic> {
    let mut doc = Doc {
        file: file.to_path_buf(),
        tables: vec![Table {
            path: Vec::new(),
            line: 0,
            entries: Vec::new(),
        }],
    };
    let mut cur = 0usize;

    for (idx, raw) in src.lines().enumerate() {
        let line = (idx + 1) as u32;
        let trimmed = raw.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if trimmed.starts_with('[') {
            let path = parse_header(trimmed, file, line)?;
            if doc.tables.iter().any(|t| t.path == path) {
                return Err(err(
                    file,
                    line,
                    format!("duplicate table `[{}]`", path.join(".")),
                ));
            }
            doc.tables.push(Table {
                path,
                line,
                entries: Vec::new(),
            });
            cur = doc.tables.len() - 1;
            continue;
        }
        // key = value
        let key_end = trimmed
            .as_bytes()
            .iter()
            .position(|&b| !is_bare_key_byte(b))
            .unwrap_or(trimmed.len());
        let key = &trimmed[..key_end];
        if key.is_empty() {
            return Err(err(file, line, format!("expected a key: `{}`", trimmed)));
        }
        let after_key = trimmed[key_end..].trim_start();
        let Some(after_eq) = after_key.strip_prefix('=') else {
            return Err(err(file, line, format!("expected `=` after key `{}`", key)));
        };
        let (value, rest) = parse_value(after_eq, file, line)?;
        expect_line_end(rest, file, line)?;
        let table = &mut doc.tables[cur];
        if table.entries.iter().any(|e| e.key == key) {
            return Err(err(file, line, format!("duplicate key `{}`", key)));
        }
        table.entries.push(Entry {
            key: key.to_string(),
            value,
            line,
        });
    }
    Ok(doc)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn parse_ok(src: &str) -> Doc {
        parse(Path::new("test.toml"), src).expect("parse should succeed")
    }

    fn parse_err(src: &str) -> Diagnostic {
        parse(Path::new("test.toml"), src).expect_err("parse should fail")
    }

    #[test]
    fn headers_and_entries() {
        let doc = parse_ok(
            "# comment\n[option.NET]\ntype = \"bool\"  # trailing\ndefault = true\n\
             [choice.arch.option.ARCH_X86_64]\nvalue = \"x86_64\"\n",
        );
        assert_eq!(doc.tables.len(), 3); // root + 2
        assert_eq!(doc.tables[1].path, vec!["option", "NET"]);
        assert_eq!(doc.tables[1].entries[0].value, Value::Str("bool".into()));
        assert_eq!(doc.tables[1].entries[1].value, Value::Bool(true));
        assert_eq!(
            doc.tables[2].path,
            vec!["choice", "arch", "option", "ARCH_X86_64"]
        );
    }

    #[test]
    fn root_entries_for_override_files() {
        let doc = parse_ok("ARCH = \"x86_64\"\nWORKER_LIMIT = 32\nBUILD_PORTS = false\n");
        assert_eq!(doc.tables[0].entries.len(), 3);
        assert_eq!(doc.tables[0].entries[1].value, Value::Int(32));
    }

    #[test]
    fn string_escapes() {
        let doc = parse_ok("k = \"a\\\"b\\\\c\\nd\"\n");
        assert_eq!(
            doc.tables[0].entries[0].value,
            Value::Str("a\"b\\c\nd".into())
        );
    }

    #[test]
    fn arrays_of_strings() {
        let doc = parse_ok("select = [\"A\", \"B\",]\nempty = []\n");
        assert_eq!(
            doc.tables[0].entries[0].value,
            Value::Array(vec!["A".into(), "B".into()])
        );
        assert_eq!(doc.tables[0].entries[1].value, Value::Array(vec![]));
    }

    #[test]
    fn arrays_of_integers() {
        let doc = parse_ok("passes = [4, 1]\nmixed = [-2,3,]\n");
        assert_eq!(doc.tables[0].entries[0].value, Value::IntList(vec![4, 1]));
        assert_eq!(doc.tables[0].entries[1].value, Value::IntList(vec![-2, 3]));
    }

    #[test]
    fn arrays_do_not_mix_element_types() {
        parse_err("k = [1, \"a\"]\n");
    }

    #[test]
    fn negative_int() {
        let doc = parse_ok("k = -12\n");
        assert_eq!(doc.tables[0].entries[0].value, Value::Int(-12));
    }

    #[test]
    fn rejects_duplicates() {
        assert_eq!(parse_err("[a.b]\n[a.b]\n").code, Code::EParse);
        assert_eq!(parse_err("k = 1\nk = 2\n").code, Code::EParse);
    }

    #[test]
    fn rejects_out_of_subset() {
        assert_eq!(parse_err("k = { a = 1 }\n").code, Code::EParse); // inline table
        assert_eq!(parse_err("k = 1.5\n").code, Code::EParse); // float
        assert_eq!(parse_err("k = 'literal'\n").code, Code::EParse); // literal string
        assert_eq!(parse_err("a.b = 1\n").code, Code::EParse); // dotted key
        assert_eq!(parse_err("k = \"unterminated\n").code, Code::EParse);
    }

    #[test]
    fn trailing_garbage_rejected() {
        assert_eq!(parse_err("k = true extra\n").code, Code::EParse);
        assert_eq!(parse_err("[a.b] extra\n").code, Code::EParse);
    }
}
