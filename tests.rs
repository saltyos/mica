// SPDX-License-Identifier: GPL-2.0-only
//! mica — end-to-end tokenize → parse → eval and version-order tests.

use super::*;
use std::cmp::Ordering;
use std::collections::BTreeMap;

fn ev(src: &str, pairs: &[(&str, Value)]) -> Result<bool, String> {
    let map: BTreeMap<String, Value> = pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.clone()))
        .collect();
    let e = parse_expr(src)?;
    eval(&e, &mut |name| map.get(name).cloned())
}

#[test]
fn precedence_and_parens() {
    let env = &[
        ("A", Value::Bool(true)),
        ("B", Value::Bool(false)),
        ("C", Value::Bool(true)),
    ];
    assert!(ev("!B && A || B", env).unwrap());
    assert!(!ev("!(B || C) && A", env).unwrap());
}

#[test]
fn comparisons_and_versions() {
    let env = &[
        ("LEVEL", Value::Str("info".into())),
        ("NCURSES", Value::Str("6.5".into())),
        ("N", Value::Int(16)),
    ];
    assert!(ev("LEVEL = \"info\"", env).unwrap());
    assert!(ev("LEVEL != \"debug\"", env).unwrap());
    assert!(ev("NCURSES >= \"6.4\"", env).unwrap());
    assert!(!ev("NCURSES < \"6.5\"", env).unwrap());
    assert!(ev("N >= 16 && N < 17", env).unwrap());
}

#[test]
fn string_atom_coercion_and_contains() {
    // A string-world lookup (flake): bare atom is `value == "true"`.
    let env = &[
        ("DEBUG", Value::Str("true".into())),
        ("QUIET", Value::Str("false".into())),
        ("PROGS", Value::Str("fluxd, fluxd-vfsd".into())),
    ];
    assert!(ev("DEBUG", env).unwrap());
    assert!(!ev("QUIET", env).unwrap());
    assert!(ev("!QUIET", env).unwrap());
    assert!(ev("PROGS contains \"fluxd-vfsd\"", env).unwrap());
    assert!(!ev("PROGS contains \"vfsd\"", env).unwrap());
}

#[test]
fn unknown_name_errors() {
    assert!(ev("MISSING", &[]).is_err());
}

#[test]
fn parse_errors() {
    assert!(parse_expr("A &&").is_err());
    assert!(parse_expr("A > B").is_err());
    assert!(parse_expr("\"lit\"").is_err());
    assert!(parse_expr("A B").is_err());
}

#[test]
fn vercmp_prerelease_rule() {
    assert_eq!(vercmp("1.0", "1.0-rc1"), Ordering::Greater);
    assert_eq!(vercmp("1.0.1", "1.0"), Ordering::Greater);
    assert_eq!(vercmp("6.5", "6.5"), Ordering::Equal);
    assert_eq!(vercmp("3.4", "4"), Ordering::Less);
}

#[test]
fn atom_collection() {
    let e = parse_expr("A && (B || C = \"x\") && D >= 2").unwrap();
    let mut out = Vec::new();
    e.atoms(&mut out);
    assert_eq!(out, vec!["A", "B", "C", "D"]);
}
