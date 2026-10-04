// SPDX-License-Identifier: LGPL-2.1-or-later
//! mica — the configuration language: option graph, resolution and artifacts
//!
//! Owns loading an option graph from a directory of `*.toml` files, its
//! structural validation, reading the persistent override file, layering
//! textual overrides over it, resolution and the emitted artifacts. The
//! buildutil engine calls [`resolve_layers`] in process; the `buildutil
//! config` commands drive the same modules for their person-facing verbs and
//! for `buildutil config resolve --out`, so both produce one set of values
//! from one set of inputs.

pub mod diag;
pub mod emit;
pub mod expr;
pub mod graph;
pub mod resolve;
pub mod toml;

use std::path::Path;

/// One layer of textual overrides over the persistent file: an
/// invocation's `-D` pairs or a configuration variant's declaration.
pub struct Layer<'a> {
    /// Names the layer in diagnostics.
    pub source: &'a str,
    pub pairs: &'a [(String, String)],
}

/// Render diagnostics one per line, as `buildutil config` prints them.
pub fn render_diagnostics(diags: &[diag::Diagnostic]) -> String {
    diags
        .iter()
        .map(|d| d.to_string())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Load the option graph in `dir` and run its structural validation.
pub fn load_graph(dir: &Path) -> Result<graph::Graph, Vec<diag::Diagnostic>> {
    let g = graph::load(dir)?;
    let errors = graph::validate(&g);
    if errors.is_empty() {
        Ok(g)
    } else {
        Err(errors)
    }
}

/// Resolve `graph` under the persistent override file, then each layer in
/// order, each replacing what an earlier one wrote under the same key. No
/// persistent file, or a missing one, is an empty first layer.
pub fn resolve_layers(
    graph: &graph::Graph,
    persistent: Option<&Path>,
    layers: &[Layer<'_>],
) -> Result<resolve::Resolved, Vec<diag::Diagnostic>> {
    let mut overrides = match persistent {
        Some(path) => resolve::load_overrides(graph, path)?,
        None => resolve::Overrides::default(),
    };
    for layer in layers {
        overrides.layer(graph, layer.pairs, layer.source)?;
    }
    resolve::resolve(graph, &overrides)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn graph() -> graph::Graph {
        let docs = vec![
            toml::parse(
                Path::new("build.toml"),
                "[choice.arch]\n\
                 default = \"ARCH_X86_64\"\n\
                 [choice.arch.option.ARCH_X86_64]\n\
                 value = \"x86_64\"\n\
                 [choice.arch.option.ARCH_AARCH64]\n\
                 value = \"aarch64\"\n\
                 [option.PASSES]\n\
                 type = \"int-list\"\n\
                 default = [4, 1]\n\
                 min = 1\n\
                 max = 8\n\
                 [option.FAST]\n\
                 type = \"bool\"\n\
                 default = \"true if PASSES contains 4\"\n",
            )
            .unwrap(),
        ];
        graph::from_docs(&docs).unwrap()
    }

    fn keyval(g: &graph::Graph, layers: &[Layer<'_>]) -> Vec<(String, String)> {
        let resolved = resolve_layers(g, None, layers).unwrap();
        emit::keyval_rows(g, &resolved)
    }

    fn value<'a>(rows: &'a [(String, String)], key: &str) -> &'a str {
        rows.iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
            .unwrap()
    }

    #[test]
    fn int_list_defaults_emit_canonically_and_feed_contains() {
        let g = graph();
        let rows = keyval(&g, &[]);
        assert_eq!(value(&rows, "PASSES"), "4,1");
        assert_eq!(value(&rows, "FAST"), "true");
    }

    #[test]
    fn later_layers_replace_earlier_ones_by_key() {
        let g = graph();
        let cli = vec![("ARCH".to_string(), "aarch64".to_string())];
        let variant = vec![("PASSES".to_string(), "2".to_string())];
        let rows = keyval(
            &g,
            &[
                Layer {
                    source: "-D",
                    pairs: &cli,
                },
                Layer {
                    source: "variant",
                    pairs: &variant,
                },
            ],
        );
        assert_eq!(value(&rows, "ARCH"), "aarch64");
        assert_eq!(value(&rows, "PASSES"), "2");
        assert_eq!(value(&rows, "FAST"), "false");

        let again = vec![("ARCH".to_string(), "x86_64".to_string())];
        let rows = keyval(
            &g,
            &[
                Layer {
                    source: "-D",
                    pairs: &cli,
                },
                Layer {
                    source: "variant",
                    pairs: &again,
                },
            ],
        );
        assert_eq!(value(&rows, "ARCH"), "x86_64");
    }

    #[test]
    fn int_list_elements_are_range_checked() {
        let g = graph();
        let bad = vec![("PASSES".to_string(), "4,9".to_string())];
        let err = resolve_layers(
            &g,
            None,
            &[Layer {
                source: "-D",
                pairs: &bad,
            }],
        )
        .unwrap_err();
        assert!(err.iter().any(|d| d.code == diag::Code::ERange));
    }
}
