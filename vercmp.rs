// SPDX-License-Identifier: GPL-2.0-only
//! mica — version-order string comparison (same rules as the port tree).

use std::cmp::Ordering;

#[derive(Debug, Clone, PartialEq, Eq)]
enum Component {
    Num(u64),
    Str(String),
}

impl Component {
    fn parse(s: &str) -> Self {
        if !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()) {
            if let Ok(n) = s.parse::<u64>() {
                return Component::Num(n);
            }
        }
        Component::Str(s.to_string())
    }

    fn is_numeric(&self) -> bool {
        matches!(self, Component::Num(_))
    }
}

impl Ord for Component {
    fn cmp(&self, other: &Self) -> Ordering {
        match (self, other) {
            (Component::Num(a), Component::Num(b)) => a.cmp(b),
            (Component::Str(a), Component::Str(b)) => a.cmp(b),
            (Component::Num(_), Component::Str(_)) => Ordering::Greater,
            (Component::Str(_), Component::Num(_)) => Ordering::Less,
        }
    }
}

impl PartialOrd for Component {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Compare two version-shaped strings: numeric components compare as u64,
/// strings lexicographically, a trailing pre-release string sorts below the
/// shorter version (`1.0-rc1 < 1.0`), a trailing numeric above (`1.0.1 >
/// 1.0`).
pub fn vercmp(a: &str, b: &str) -> Ordering {
    let parse = |s: &str| -> Vec<Component> {
        s.trim()
            .split(|c: char| c == '.' || c == '-')
            .filter(|s| !s.is_empty())
            .map(Component::parse)
            .collect()
    };
    let av = parse(a);
    let bv = parse(b);
    let len = std::cmp::min(av.len(), bv.len());
    for i in 0..len {
        match av[i].cmp(&bv[i]) {
            Ordering::Equal => continue,
            ord => return ord,
        }
    }
    match av.len().cmp(&bv.len()) {
        Ordering::Equal => Ordering::Equal,
        Ordering::Greater => {
            if av[len].is_numeric() {
                Ordering::Greater
            } else {
                Ordering::Less
            }
        }
        Ordering::Less => {
            if bv[len].is_numeric() {
                Ordering::Less
            } else {
                Ordering::Greater
            }
        }
    }
}
