// SPDX-License-Identifier: LGPL-2.1-or-later
//! mica — configuration resolution
//!
//! Pipeline (after structural validation): apply the override file, resolve
//! visibility (hidden = fixed off; an override on a hidden option is a hard
//! error), evaluate defaults (after visibility), enforce choices (exactly
//! one active member; restoring an overridden-off default warns), run the
//! select fixpoint (forcing a hidden target, a target of a conflicting
//! active choice, or an explicitly overridden-off target fails loud), and
//! stabilize the outer loop since selects can flip visibility.

use crate::config::diag::{Code, Diagnostic};
use crate::config::expr::{self, CmpVal, EvalCtx};
use crate::config::graph::{Graph, Type};
use crate::config::toml::{self, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    Default,
    Override,
    Select,
    ChoiceDefault,
    Hidden,
}

impl Origin {
    pub fn as_str(self) -> &'static str {
        match self {
            Origin::Default => "default",
            Origin::Override => "override",
            Origin::Select => "select",
            Origin::ChoiceDefault => "choice-default",
            Origin::Hidden => "hidden",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ResolvedOpt {
    pub value: Value,
    pub origin: Origin,
    pub visible: bool,
}

#[derive(Debug, Clone)]
pub struct ChoiceState {
    pub visible: bool,
    pub active: Option<String>,
}

#[derive(Debug)]
pub struct Resolved {
    pub values: BTreeMap<String, ResolvedOpt>,
    pub choices: BTreeMap<String, ChoiceState>,
    pub warnings: Vec<Diagnostic>,
}

#[derive(Debug, Clone)]
pub struct OverrideEntry {
    pub value: Value,
    pub line: u32,
    /// The key as written in the override file (a choice-label override
    /// translates to a member entry; this preserves the original spelling
    /// for diagnostics).
    pub written_as: String,
}

#[derive(Debug, Default)]
pub struct Overrides {
    pub file: PathBuf,
    pub entries: BTreeMap<String, OverrideEntry>,
}

/// Load and validate the override file (Kconfig `.config` analog): flat
/// `KEY = value` pairs. A missing file is an empty override set.
pub fn load_overrides(graph: &Graph, path: &Path) -> Result<Overrides, Vec<Diagnostic>> {
    let mut ov = Overrides {
        file: path.to_path_buf(),
        entries: BTreeMap::new(),
    };
    let src = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(ov),
        Err(e) => {
            return Err(vec![Diagnostic::new(
                Code::EParse,
                format!("cannot read {}: {}", path.display(), e),
            )]);
        }
    };
    let doc = toml::parse(path, &src).map_err(|d| vec![d])?;
    let mut errors = Vec::new();
    for table in &doc.tables {
        if !table.path.is_empty() {
            errors.push(
                Diagnostic::new(
                    Code::EParse,
                    "override files take flat KEY = value pairs, not tables",
                )
                .at(path, table.line),
            );
            continue;
        }
        for e in &table.entries {
            match check_override(graph, &e.key, &e.value, path, e.line) {
                Ok((key, value)) => {
                    if let Some(prev) = ov.entries.get(&key) {
                        errors.push(
                            Diagnostic::new(
                                Code::EDup,
                                format!(
                                    "`{}` conflicts with earlier override `{}`",
                                    e.key, prev.written_as
                                ),
                            )
                            .at(path, e.line),
                        );
                    } else {
                        ov.entries.insert(
                            key,
                            OverrideEntry {
                                value,
                                line: e.line,
                                written_as: e.key.clone(),
                            },
                        );
                    }
                }
                Err(d) => errors.push(d),
            }
        }
    }
    if errors.is_empty() {
        Ok(ov)
    } else {
        Err(errors)
    }
}

/// The textual form of an override value, as `-D<KEY>=<value>` and a
/// configuration variant's declaration write it: `true`/`false` for a
/// boolean, a decimal for an integer, comma-separated decimals for an
/// integer list, the member's value for a choice key, the raw text for a
/// string.
fn parse_override_text(graph: &Graph, key: &str, text: &str) -> Result<Value, Diagnostic> {
    let Some(def) = graph.options.get(key) else {
        // Choice keys and unknown keys are validated by `check_override`.
        return Ok(Value::Str(text.to_string()));
    };
    let bad = |what: &str| {
        Diagnostic::new(
            Code::EType,
            format!("`{}` is {}, got `{}` ({})", key, def.ty.name(), text, what),
        )
    };
    match def.ty {
        Type::Bool | Type::ChoiceMember => match text {
            "true" => Ok(Value::Bool(true)),
            "false" => Ok(Value::Bool(false)),
            _ => Err(bad("expected true or false")),
        },
        Type::Int => text
            .trim()
            .parse::<i64>()
            .map(Value::Int)
            .map_err(|_| bad("expected a decimal integer")),
        Type::IntList => {
            let trimmed = text.trim();
            if trimmed.is_empty() {
                return Ok(Value::IntList(Vec::new()));
            }
            trimmed
                .split(',')
                .map(|item| item.trim().parse::<i64>())
                .collect::<Result<Vec<_>, _>>()
                .map(Value::IntList)
                .map_err(|_| bad("expected comma-separated decimal integers"))
        }
        Type::Str => Ok(Value::Str(text.to_string())),
    }
}

impl Overrides {
    /// Layer textual overrides over this set: each key replaces the entry
    /// an earlier layer wrote under the same key, so the persistent file,
    /// then `-D`, then a configuration variant each win over the one
    /// before. `source` names the layer in diagnostics.
    pub fn layer(
        &mut self,
        graph: &Graph,
        pairs: &[(String, String)],
        source: &str,
    ) -> Result<(), Vec<Diagnostic>> {
        let origin = PathBuf::from(source);
        let mut errors = Vec::new();
        for (key, text) in pairs {
            // A textual layer has no line numbers; the source names it.
            let checked = parse_override_text(graph, key, text)
                .and_then(|value| check_override(graph, key, &value, &origin, 0))
                .map_err(|mut d| {
                    d.file = Some(origin.clone());
                    d.line = None;
                    d
                });
            match checked {
                Ok((target, value)) => {
                    self.entries.retain(|name, entry| {
                        entry.written_as != *key && *name != target
                    });
                    self.entries.insert(
                        target,
                        OverrideEntry {
                            value,
                            line: 0,
                            written_as: key.clone(),
                        },
                    );
                }
                Err(d) => errors.push(d),
            }
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }
}

/// Comma-separated decimals without spaces: the canonical text of an
/// integer list in emitted artifacts and predicates.
pub fn join_ints(items: &[i64]) -> String {
    items
        .iter()
        .map(|n| n.to_string())
        .collect::<Vec<_>>()
        .join(",")
}

/// Validate one override entry; returns the (possibly translated) target
/// option name and value. A choice-label override (`ARCH = "aarch64"`)
/// translates to `Bool(true)` on the matching member.
fn check_override(
    graph: &Graph,
    key: &str,
    value: &Value,
    file: &Path,
    line: u32,
) -> Result<(String, Value), Diagnostic> {
    if let Some(def) = graph.options.get(key) {
        if def.computed {
            return Err(Diagnostic::new(
                Code::EComputedOverride,
                format!("`{}` is computed and cannot be overridden", key),
            )
            .at(file, line));
        }
        // An empty array carries no element type; for an int-list it is
        // the empty list.
        let value = match (def.ty, value) {
            (Type::IntList, Value::Array(items)) if items.is_empty() => Value::IntList(Vec::new()),
            _ => value.clone(),
        };
        let ok = matches!(
            (def.ty, &value),
            (Type::Bool, Value::Bool(_))
                | (Type::ChoiceMember, Value::Bool(_))
                | (Type::Int, Value::Int(_))
                | (Type::IntList, Value::IntList(_))
                | (Type::Str, Value::Str(_))
        );
        if !ok {
            return Err(Diagnostic::new(
                Code::EType,
                format!("`{}` is {}, got {}", key, def.ty.name(), value.type_name()),
            )
            .at(file, line));
        }
        let elements: &[i64] = match &value {
            Value::Int(n) => std::slice::from_ref(n),
            Value::IntList(items) => items,
            _ => &[],
        };
        for n in elements {
            if def.out_of_range(*n) {
                return Err(Diagnostic::new(
                    Code::ERange,
                    format!(
                        "`{}` = {} is outside [{}, {}]",
                        key,
                        n,
                        def.min.map_or("-".to_string(), |m| m.to_string()),
                        def.max.map_or("-".to_string(), |m| m.to_string())
                    ),
                )
                .at(file, line));
            }
        }
        return Ok((key.to_string(), value));
    }
    if let Some(choice) = graph.choice_for_key(key) {
        let Value::Str(label) = value else {
            return Err(Diagnostic::new(
                Code::EType,
                format!("`{}` selects a choice member by its string value", key),
            )
            .at(file, line));
        };
        for member in &choice.members {
            let mdef = &graph.options[member];
            if mdef.value_label.as_deref() == Some(label.as_str()) {
                return Ok((member.clone(), Value::Bool(true)));
            }
        }
        let labels: Vec<String> = choice
            .members
            .iter()
            .filter_map(|m| graph.options[m].value_label.clone())
            .collect();
        return Err(Diagnostic::new(
            Code::EType,
            format!(
                "`{}` has no member valued \"{}\" (one of: {})",
                key,
                label,
                labels.join(", ")
            ),
        )
        .at(file, line));
    }
    Err(Diagnostic::new(Code::EUnknownOverride, format!("unknown option `{}`", key)).at(file, line))
}

// ---------------------------------------------------------------------------
// One resolution pass (memoized; the reference graph is statically acyclic)

struct Pass<'g> {
    graph: &'g Graph,
    ov: &'g BTreeMap<String, OverrideEntry>,
    /// Select-forced options with their select chains (selector path).
    forced: &'g BTreeMap<String, Vec<String>>,
    opts: BTreeMap<String, ResolvedOpt>,
    choices: BTreeMap<String, ChoiceState>,
    in_progress: BTreeSet<String>,
    warnings: Vec<Diagnostic>,
    errors: Vec<Diagnostic>,
}

impl<'g> Pass<'g> {
    fn new(
        graph: &'g Graph,
        ov: &'g BTreeMap<String, OverrideEntry>,
        forced: &'g BTreeMap<String, Vec<String>>,
    ) -> Self {
        Pass {
            graph,
            ov,
            forced,
            opts: BTreeMap::new(),
            choices: BTreeMap::new(),
            in_progress: BTreeSet::new(),
            warnings: Vec::new(),
            errors: Vec::new(),
        }
    }

    fn eval(&mut self, e: &expr::Expr) -> bool {
        match expr::eval(e, self) {
            Ok(b) => b,
            Err(d) => {
                self.errors.push(d);
                false
            }
        }
    }

    fn opt(&mut self, name: &str) -> ResolvedOpt {
        if let Some(r) = self.opts.get(name) {
            return r.clone();
        }
        let Some(def) = self.graph.options.get(name) else {
            self.errors.push(Diagnostic::new(
                Code::EUnknownRef,
                format!("unknown option `{}`", name),
            ));
            return ResolvedOpt {
                value: Value::Bool(false),
                origin: Origin::Hidden,
                visible: false,
            };
        };
        if !self.in_progress.insert(name.to_string()) {
            self.errors.push(Diagnostic::new(
                Code::EDepCycle,
                format!("`{}` participates in a cycle", name),
            ));
            return ResolvedOpt {
                value: def.off_value(),
                origin: Origin::Hidden,
                visible: false,
            };
        }

        let mut visible = match &def.depends_on {
            Some(e) => {
                let e = e.clone();
                self.eval(&e)
            }
            None => true,
        };
        if let Some(choice) = def.parent_choice.clone() {
            visible = visible && self.choice(&choice).visible;
        }

        let resolved = if !visible {
            ResolvedOpt {
                value: def.off_value(),
                origin: Origin::Hidden,
                visible: false,
            }
        } else if def.ty == Type::ChoiceMember {
            let choice = def
                .parent_choice
                .clone()
                .expect("members have a parent choice");
            let state = self.choice(&choice);
            let active = state.active.as_deref() == Some(name);
            let origin = if self.forced.contains_key(name) {
                Origin::Select
            } else if self.ov.contains_key(name) {
                Origin::Override
            } else if active {
                Origin::ChoiceDefault
            } else {
                Origin::Default
            };
            ResolvedOpt {
                value: Value::Bool(active),
                origin,
                visible: true,
            }
        } else if self.forced.contains_key(name) {
            ResolvedOpt {
                value: Value::Bool(true),
                origin: Origin::Select,
                visible: true,
            }
        } else if let Some(entry) = self.ov.get(name) {
            ResolvedOpt {
                value: entry.value.clone(),
                origin: Origin::Override,
                visible: true,
            }
        } else if let Some(spec) = def.default.clone() {
            let value = match expr::eval_default(&spec, self) {
                Ok(Some(v)) => v,
                Ok(None) => def.off_value(),
                Err(d) => {
                    self.errors.push(d);
                    def.off_value()
                }
            };
            ResolvedOpt {
                value,
                origin: Origin::Default,
                visible: true,
            }
        } else {
            ResolvedOpt {
                value: def.off_value(),
                origin: Origin::Default,
                visible: true,
            }
        };

        self.in_progress.remove(name);
        self.opts.insert(name.to_string(), resolved.clone());
        resolved
    }

    fn choice(&mut self, name: &str) -> ChoiceState {
        if let Some(s) = self.choices.get(name) {
            return s.clone();
        }
        let Some(def) = self.graph.choices.get(name) else {
            self.errors.push(Diagnostic::new(
                Code::EUnknownRef,
                format!("unknown choice `{}`", name),
            ));
            return ChoiceState {
                visible: false,
                active: None,
            };
        };
        let key = format!("choice:{}", name);
        if !self.in_progress.insert(key.clone()) {
            self.errors.push(Diagnostic::new(
                Code::EDepCycle,
                format!("choice `{}` participates in a cycle", name),
            ));
            return ChoiceState {
                visible: false,
                active: None,
            };
        }

        let visible = match &def.depends_on {
            Some(e) => {
                let e = e.clone();
                self.eval(&e)
            }
            None => true,
        };

        let state = if !visible {
            ChoiceState {
                visible: false,
                active: None,
            }
        } else {
            // Candidates: members explicitly overridden on, plus select-forced
            // members. (Reads the raw maps — never recurses into member values.)
            let mut candidates: Vec<&String> = Vec::new();
            for member in &def.members {
                let overridden_on = self
                    .ov
                    .get(member)
                    .is_some_and(|e| e.value == Value::Bool(true));
                if overridden_on || self.forced.contains_key(member) {
                    candidates.push(member);
                }
            }
            candidates.dedup();
            if candidates.len() > 1 {
                self.errors.push(
                    Diagnostic::new(
                        Code::EChoiceMulti,
                        format!(
                            "choice `{}` has multiple members forced on: {}",
                            name,
                            candidates
                                .iter()
                                .map(|s| s.as_str())
                                .collect::<Vec<_>>()
                                .join(", ")
                        ),
                    )
                    .at(&def.file, def.line),
                );
            }
            let active = if let Some(first) = candidates.first() {
                (*first).clone()
            } else {
                let default_off = self
                    .ov
                    .get(&def.default_member)
                    .is_some_and(|e| e.value == Value::Bool(false));
                if default_off {
                    self.warnings.push(
                        Diagnostic::new(
                            Code::WChoiceRestored,
                            format!(
                                "choice `{}`: default member `{}` was switched off with no \
                                 replacement; restoring the default",
                                name, def.default_member
                            ),
                        )
                        .at(&def.file, def.line),
                    );
                }
                def.default_member.clone()
            };
            ChoiceState {
                visible: true,
                active: Some(active),
            }
        };

        self.in_progress.remove(&key);
        self.choices.insert(name.to_string(), state.clone());
        state
    }
}

impl<'g> EvalCtx for Pass<'g> {
    fn lookup(&mut self, name: &str) -> Result<CmpVal, Diagnostic> {
        if let Some(def) = self.graph.options.get(name) {
            let ty = def.ty;
            let r = self.opt(name);
            return Ok(match (ty, r.value) {
                (Type::Bool | Type::ChoiceMember, Value::Bool(b)) => CmpVal::Bool(b),
                (Type::Int, Value::Int(n)) => CmpVal::Int(n),
                // Predicates see an int-list in its emitted form, the one
                // the engine's projections read, so `contains` agrees in both.
                (Type::IntList, Value::IntList(items)) => CmpVal::Str(join_ints(&items)),
                (Type::Str, Value::Str(s)) => CmpVal::Str(s),
                (_, v) => {
                    return Err(Diagnostic::new(
                        Code::EType,
                        format!("`{}` resolved to mismatched value {:?}", name, v),
                    ));
                }
            });
        }
        if self.graph.choices.contains_key(name) {
            let state = self.choice(name);
            let label = state
                .active
                .as_ref()
                .and_then(|m| self.graph.options[m].value_label.clone())
                .unwrap_or_default();
            return Ok(CmpVal::Str(label));
        }
        Err(Diagnostic::new(
            Code::EUnknownRef,
            format!("unknown name `{}`", name),
        ))
    }
}

// ---------------------------------------------------------------------------
// Outer driver: select fixpoint + stabilization

pub fn resolve(graph: &Graph, ov: &Overrides) -> Result<Resolved, Vec<Diagnostic>> {
    let mut forced: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let cap = graph.options.len() + 1;

    for _iter in 0..=cap {
        let mut pass = Pass::new(graph, &ov.entries, &forced);
        let names: Vec<String> = graph.options.keys().cloned().collect();
        for name in &names {
            pass.opt(name);
        }
        let choice_names: Vec<String> = graph.choices.keys().cloned().collect();
        for name in &choice_names {
            pass.choice(name);
        }
        if !pass.errors.is_empty() {
            return Err(pass.errors);
        }

        // Select scan: every on bool with a select list forces its targets.
        // New forces are collected and merged into `forced` after the scan
        // (`pass` holds a shared borrow of `forced` until its last use).
        let mut errors: Vec<Diagnostic> = Vec::new();
        let mut pending: Vec<(String, Vec<String>)> = Vec::new();
        let mut pending_set: BTreeSet<String> = BTreeSet::new();
        let mut pending_choice_member: BTreeMap<String, String> = BTreeMap::new();
        for name in &names {
            let def = &graph.options[name];
            if def.select.is_empty() {
                continue;
            }
            if pass.opts[name].value != Value::Bool(true) {
                continue;
            }
            let chain_base = forced.get(name).cloned().unwrap_or_default();
            for target in &def.select {
                let state = &pass.opts[target];
                if state.value == Value::Bool(true)
                    || forced.contains_key(target)
                    || pending_set.contains(target)
                {
                    continue;
                }
                let mut chain = chain_base.clone();
                chain.push(name.clone());
                chain.push(target.clone());
                if !state.visible {
                    errors.push(
                        Diagnostic::new(
                            Code::ESelectHidden,
                            format!(
                                "`{}` selects `{}`, which is hidden (its dependencies are \
                                 unsatisfied)",
                                name, target
                            ),
                        )
                        .with_chain(chain),
                    );
                    continue;
                }
                if ov
                    .entries
                    .get(target)
                    .is_some_and(|e| e.value == Value::Bool(false))
                {
                    errors.push(
                        Diagnostic::new(
                            Code::ESelectVsOverride,
                            format!(
                                "`{}` selects `{}`, which the override file switches off",
                                name, target
                            ),
                        )
                        .with_chain(chain),
                    );
                    continue;
                }
                let tdef = &graph.options[target];
                if let Some(cname) = &tdef.parent_choice {
                    // A member pinned by the user, an earlier force, or a
                    // same-scan pending force conflicts with forcing another
                    // member of the same choice on.
                    let pinned_active = pass.choices[cname].active.clone().filter(|active| {
                        ov.entries
                            .get(active)
                            .is_some_and(|e| e.value == Value::Bool(true))
                            || forced.contains_key(active)
                    });
                    let conflicting =
                        pinned_active.filter(|active| active != target).or_else(|| {
                            pending_choice_member
                                .get(cname)
                                .filter(|m| *m != target)
                                .cloned()
                        });
                    if let Some(active) = conflicting {
                        errors.push(
                            Diagnostic::new(
                                Code::ESelectChoiceConflict,
                                format!(
                                    "`{}` selects `{}`, but choice `{}` already has `{}` \
                                     forced on",
                                    name, target, cname, active
                                ),
                            )
                            .with_chain(chain),
                        );
                        continue;
                    }
                    pending_choice_member.insert(cname.clone(), target.clone());
                }
                pending_set.insert(target.clone());
                pending.push((target.clone(), chain));
            }
        }
        if !errors.is_empty() {
            return Err(errors);
        }

        if pending.is_empty() {
            // Converged. Post-convergence: an override on a hidden option is
            // a hard error (the user asked for something that cannot be set).
            let mut hidden_errors = Vec::new();
            for (key, entry) in &ov.entries {
                if !pass.opts[key].visible {
                    hidden_errors.push(
                        Diagnostic::new(
                            Code::EHiddenOverride,
                            format!(
                                "override `{}` targets `{}`, which is hidden (its dependencies \
                                 are unsatisfied)",
                                entry.written_as, key
                            ),
                        )
                        .at(&ov.file, entry.line),
                    );
                }
            }
            if !hidden_errors.is_empty() {
                return Err(hidden_errors);
            }
            return Ok(Resolved {
                values: pass.opts,
                choices: pass.choices,
                warnings: pass.warnings,
            });
        }

        drop(pass);
        for (target, chain) in pending {
            forced.insert(target, chain);
        }
    }

    Err(vec![Diagnostic::new(
        Code::EUnstable,
        format!("resolution did not stabilize within {} iterations", cap + 1),
    )])
}

/// Config-space generation mode for `buildutil config resolve --allno|--allyes|--rand`.
#[derive(Clone, Copy)]
pub enum ConfigMode {
    AllNo,
    AllYes,
    Rand(u64),
}

/// splitmix64 — a tiny, dependency-free deterministic PRNG for `--rand`.
fn next_rand(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Resolve a whole-config-space assignment (allno / allyes / seeded rand):
/// synthesize an override over the visible, non-computed options, then run
/// normal resolution, dropping any override the resolver rejects and
/// retrying within a bounded budget — so the emitted config is always
/// valid-by-construction. `E_RAND_UNSAT` if the budget is exhausted.
pub fn resolve_config_space(graph: &Graph, mode: ConfigMode) -> Result<Resolved, Vec<Diagnostic>> {
    // Base resolution establishes visibility for the synthesis step.
    let base = resolve(graph, &Overrides::default())?;
    let mut rng = match mode {
        ConfigMode::Rand(seed) => seed ^ 0x9E37_79B9_7F4A_7C15,
        _ => 0,
    };

    let mut ov = Overrides::default();
    let mut put = |name: &str, value: Value| {
        ov.entries.insert(
            name.to_string(),
            OverrideEntry {
                value,
                line: 0,
                written_as: name.to_string(),
            },
        );
    };

    for name in &graph.decl_order {
        let Some(def) = graph.options.get(name) else {
            continue;
        };
        if def.computed || def.ty == Type::ChoiceMember {
            continue; // choice members are driven through their choice
        }
        let Some(state) = base.values.get(name) else {
            continue;
        };
        if !state.visible || matches!(state.origin, Origin::Select) {
            continue; // hidden or select-forced: leave to resolution
        }
        match def.ty {
            Type::Bool => {
                let v = match mode {
                    ConfigMode::AllNo => false,
                    ConfigMode::AllYes => true,
                    ConfigMode::Rand(_) => next_rand(&mut rng) & 1 == 1,
                };
                put(name, Value::Bool(v));
            }
            Type::Int => match mode {
                ConfigMode::AllNo => put(name, Value::Int(def.min.unwrap_or(0))),
                ConfigMode::AllYes => {
                    if let Some(max) = def.max {
                        put(name, Value::Int(max));
                    }
                }
                ConfigMode::Rand(_) => {
                    if let (Some(lo), Some(hi)) = (def.min, def.max) {
                        if hi >= lo {
                            let span = (hi - lo + 1) as u64;
                            put(name, Value::Int(lo + (next_rand(&mut rng) % span) as i64));
                        }
                    }
                }
            },
            // Strings and integer lists keep their default.
            Type::Str | Type::IntList | Type::ChoiceMember => {}
        }
    }

    // For rand, force a random member of each visible choice.
    if let ConfigMode::Rand(_) = mode {
        for (cname, cstate) in &base.choices {
            if !cstate.visible {
                continue;
            }
            let choice = &graph.choices[cname];
            if choice.members.is_empty() {
                continue;
            }
            let pick = (next_rand(&mut rng) as usize) % choice.members.len();
            ov.entries.insert(
                choice.members[pick].clone(),
                OverrideEntry {
                    value: Value::Bool(true),
                    line: 0,
                    written_as: choice.members[pick].clone(),
                },
            );
        }
    }

    // Drop-and-retry: any override the resolver rejects (now hidden, choice
    // conflict, select clash) is removed and resolution retried.
    let budget = graph.options.len() + 2;
    for _ in 0..budget {
        match resolve(graph, &ov) {
            Ok(r) => return Ok(r),
            Err(diags) => {
                let before = ov.entries.len();
                ov.entries.retain(|k, _| {
                    !diags
                        .iter()
                        .any(|d| d.message.contains(&format!("`{}`", k)))
                });
                if ov.entries.len() == before {
                    return Err(vec![Diagnostic::new(
                        Code::ERandUnsat,
                        "config-space resolution failed on an error not tied to a droppable override",
                    )]);
                }
            }
        }
    }
    Err(vec![Diagnostic::new(
        Code::ERandUnsat,
        "config-space resolution exceeded the repair budget",
    )])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::graph;
    use std::path::Path;

    fn setup(graph_src: &str, config_src: &str) -> Result<Resolved, Vec<Diagnostic>> {
        let doc = toml::parse(Path::new("g.toml"), graph_src).map_err(|d| vec![d])?;
        let g = graph::from_docs(&[doc])?;
        let errs = graph::validate(&g);
        if !errs.is_empty() {
            return Err(errs);
        }
        let cfg_doc = toml::parse(Path::new("config"), config_src).map_err(|d| vec![d])?;
        // Reuse load path semantics by validating entries directly.
        let mut ov = Overrides {
            file: "config".into(),
            entries: BTreeMap::new(),
        };
        let mut errors = Vec::new();
        for e in &cfg_doc.tables[0].entries {
            match check_override(&g, &e.key, &e.value, Path::new("config"), e.line) {
                Ok((k, v)) => {
                    ov.entries.insert(
                        k,
                        OverrideEntry {
                            value: v,
                            line: e.line,
                            written_as: e.key.clone(),
                        },
                    );
                }
                Err(d) => errors.push(d),
            }
        }
        if !errors.is_empty() {
            return Err(errors);
        }
        resolve(&g, &ov)
    }

    const BASE: &str = r#"
[option.BUILD_USERLAND]
type = "bool"
default = true

[option.BUILD_PORTS]
type = "bool"
default = false
depends_on = "BUILD_USERLAND"

[option.NET]
type = "bool"
default = false
select = ["NETDRV"]

[option.NETDRV]
type = "bool"
default = false

[option.DEBUG_SERIAL]
type = "bool"
default = true

[option.NO_SERIAL_OUTPUT]
type = "bool"
computed = true
default = "true if !DEBUG_SERIAL else false"

[choice.kernel_log_level]
default = "KERNEL_LOG_LEVEL_INFO"
  [choice.kernel_log_level.option.KERNEL_LOG_LEVEL_ERROR]
  [choice.kernel_log_level.option.KERNEL_LOG_LEVEL_INFO]
  [choice.kernel_log_level.option.KERNEL_LOG_LEVEL_DEBUG]

[option.KLOG_DEBUG]
type = "bool"
computed = true
default = "true if KERNEL_LOG_LEVEL_DEBUG else false"

[option.WORKER_LIMIT]
type = "int"
default = 16
min = 1
max = 256
"#;

    #[test]
    fn defaults_and_overrides() {
        let r = setup(BASE, "WORKER_LIMIT = 32\n").unwrap();
        assert_eq!(r.values["WORKER_LIMIT"].value, Value::Int(32));
        assert_eq!(r.values["WORKER_LIMIT"].origin, Origin::Override);
        assert_eq!(r.values["BUILD_USERLAND"].value, Value::Bool(true));
        assert_eq!(r.values["NO_SERIAL_OUTPUT"].value, Value::Bool(false));
        assert_eq!(
            r.choices["kernel_log_level"].active.as_deref(),
            Some("KERNEL_LOG_LEVEL_INFO")
        );
        assert_eq!(r.values["KERNEL_LOG_LEVEL_INFO"].value, Value::Bool(true));
        assert_eq!(r.values["KERNEL_LOG_LEVEL_DEBUG"].value, Value::Bool(false));
    }

    #[test]
    fn derived_option_follows_source() {
        let r = setup(BASE, "DEBUG_SERIAL = false\n").unwrap();
        assert_eq!(r.values["NO_SERIAL_OUTPUT"].value, Value::Bool(true));
    }

    #[test]
    fn choice_label_override() {
        let r = setup(BASE, "KERNEL_LOG_LEVEL = \"debug\"\n").unwrap();
        assert_eq!(
            r.choices["kernel_log_level"].active.as_deref(),
            Some("KERNEL_LOG_LEVEL_DEBUG")
        );
        assert_eq!(r.values["KLOG_DEBUG"].value, Value::Bool(true));
    }

    #[test]
    fn choice_member_bool_override() {
        let r = setup(BASE, "KERNEL_LOG_LEVEL_ERROR = true\n").unwrap();
        assert_eq!(
            r.choices["kernel_log_level"].active.as_deref(),
            Some("KERNEL_LOG_LEVEL_ERROR")
        );
    }

    #[test]
    fn choice_multi_rejected() {
        let errs = setup(
            BASE,
            "KERNEL_LOG_LEVEL_ERROR = true\nKERNEL_LOG_LEVEL_DEBUG = true\n",
        )
        .unwrap_err();
        assert!(errs.iter().any(|d| d.code == Code::EChoiceMulti));
    }

    #[test]
    fn choice_default_restored_with_warning() {
        let r = setup(BASE, "KERNEL_LOG_LEVEL_INFO = false\n").unwrap();
        assert_eq!(
            r.choices["kernel_log_level"].active.as_deref(),
            Some("KERNEL_LOG_LEVEL_INFO")
        );
        assert!(r.warnings.iter().any(|d| d.code == Code::WChoiceRestored));
    }

    #[test]
    fn select_forces_target() {
        let r = setup(BASE, "NET = true\n").unwrap();
        assert_eq!(r.values["NETDRV"].value, Value::Bool(true));
        assert_eq!(r.values["NETDRV"].origin, Origin::Select);
    }

    #[test]
    fn select_vs_override_fails_loud() {
        let errs = setup(BASE, "NET = true\nNETDRV = false\n").unwrap_err();
        let e = errs
            .iter()
            .find(|d| d.code == Code::ESelectVsOverride)
            .unwrap();
        assert_eq!(e.chain, vec!["NET", "NETDRV"]);
    }

    #[test]
    fn select_hidden_fails_with_chain() {
        let src = format!(
            "{}\n[option.GATED]\ntype = \"bool\"\ndepends_on = \"BUILD_PORTS\"\n\
             [option.WANTS_GATED]\ntype = \"bool\"\ndefault = false\nselect = [\"GATED\"]\n",
            BASE
        );
        let errs = setup(&src, "WANTS_GATED = true\n").unwrap_err();
        let e = errs.iter().find(|d| d.code == Code::ESelectHidden).unwrap();
        assert_eq!(e.chain, vec!["WANTS_GATED", "GATED"]);
    }

    #[test]
    fn transitive_select_chain() {
        let src = format!(
            "{}\n[option.TOP]\ntype = \"bool\"\ndefault = false\nselect = [\"MID\"]\n\
             [option.MID]\ntype = \"bool\"\ndefault = false\nselect = [\"NETDRV\"]\n",
            BASE
        );
        let r = setup(&src, "TOP = true\n").unwrap();
        assert_eq!(r.values["MID"].origin, Origin::Select);
        assert_eq!(r.values["NETDRV"].value, Value::Bool(true));
    }

    #[test]
    fn hidden_override_rejected() {
        let errs = setup(BASE, "BUILD_USERLAND = false\nBUILD_PORTS = true\n").unwrap_err();
        assert!(errs.iter().any(|d| d.code == Code::EHiddenOverride));
    }

    #[test]
    fn hidden_option_fixed_off() {
        let r = setup(BASE, "BUILD_USERLAND = false\n").unwrap();
        assert_eq!(r.values["BUILD_PORTS"].value, Value::Bool(false));
        assert_eq!(r.values["BUILD_PORTS"].origin, Origin::Hidden);
        assert!(!r.values["BUILD_PORTS"].visible);
    }

    #[test]
    fn computed_override_rejected() {
        let errs = setup(BASE, "NO_SERIAL_OUTPUT = true\n").unwrap_err();
        assert!(errs.iter().any(|d| d.code == Code::EComputedOverride));
    }

    #[test]
    fn unknown_and_range_rejected() {
        let errs = setup(BASE, "NOPE = true\n").unwrap_err();
        assert!(errs.iter().any(|d| d.code == Code::EUnknownOverride));
        let errs = setup(BASE, "WORKER_LIMIT = 400\n").unwrap_err();
        assert!(errs.iter().any(|d| d.code == Code::ERange));
    }

    #[test]
    fn select_flips_visibility_of_dependents() {
        // NET selects NETDRV; GATED depends on NETDRV — forcing NETDRV must
        // make GATED visible in a later stabilization iteration.
        let src = format!(
            "{}\n[option.GATED2]\ntype = \"bool\"\ndefault = true\ndepends_on = \"NETDRV\"\n",
            BASE
        );
        let r = setup(&src, "NET = true\n").unwrap();
        assert_eq!(r.values["GATED2"].value, Value::Bool(true));
        assert!(r.values["GATED2"].visible);
    }
}
