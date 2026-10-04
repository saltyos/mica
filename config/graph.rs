// SPDX-License-Identifier: LGPL-2.1-or-later
//! mica — option graph: definitions, loading, structural validation
//!
//! Graph files live in a directory of `*.toml` files (sorted by filename;
//! options within a file in declaration order). Table shapes:
//! `[option.NAME]`, `[choice.name]`, `[choice.name.option.MEMBER]`.
//! A choice's header must precede its member tables.

use crate::config::diag::{Code, Diagnostic};
use crate::config::expr::{self, DefaultSpec, Expr};
use crate::config::toml::{self, Doc, Table, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Type {
    Bool,
    Str,
    Int,
    /// A list of integers, each within the option's bounds.
    IntList,
    ChoiceMember,
}

impl Type {
    pub fn name(self) -> &'static str {
        match self {
            Type::Bool => "bool",
            Type::Str => "string",
            Type::Int => "int",
            Type::IntList => "int-list",
            Type::ChoiceMember => "choice-member",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    Kernel,
    Userland,
    None,
}

#[derive(Debug, Clone)]
pub struct OptionDef {
    pub name: String,
    pub ty: Type,
    pub default: Option<DefaultSpec>,
    pub depends_on: Option<Expr>,
    pub select: Vec<String>,
    /// Derived option: computed from its guard default; user override is a
    /// hard error.
    pub computed: bool,
    pub min: Option<i64>,
    pub max: Option<i64>,
    /// Short display name for interactive front ends; the option name when
    /// absent.
    pub title: Option<String>,
    pub help: Option<String>,
    /// Emit this rustc `--cfg` name when the option resolves true.
    pub rust_cfg: Option<String>,
    /// For comma-list string options: emit `<prefix><item>` per list item.
    pub rust_cfg_each_prefix: Option<String>,
    pub scope: Scope,
    pub parent_choice: Option<String>,
    /// Choice-member external value (e.g. "info" for KERNEL_LOG_LEVEL_INFO).
    pub value_label: Option<String>,
    /// Organization-only menu membership (`[menu.NAME]`); no value semantics.
    pub menu: Option<String>,
    pub file: PathBuf,
    pub line: u32,
}

impl OptionDef {
    /// Whether `n` lies outside the inclusive bounds; for an int-list the
    /// bounds apply to each element.
    pub fn out_of_range(&self, n: i64) -> bool {
        self.min.is_some_and(|m| n < m) || self.max.is_some_and(|m| n > m)
    }

    pub fn off_value(&self) -> Value {
        match self.ty {
            Type::Bool | Type::ChoiceMember => Value::Bool(false),
            Type::Int => Value::Int(0),
            Type::IntList => Value::IntList(Vec::new()),
            Type::Str => Value::Str(String::new()),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ChoiceDef {
    pub name: String,
    pub title: Option<String>,
    pub default_member: String,
    /// Members in declaration order.
    pub members: Vec<String>,
    pub depends_on: Option<Expr>,
    pub help: Option<String>,
    pub menu: Option<String>,
    pub file: PathBuf,
    pub line: u32,
}

/// Organization-only grouping (`[menu.NAME]`) — title, optional parent for
/// nesting, optional `visible_when` guard. No value semantics; consumed by
/// the TUI, `buildutil config docs`, and `buildutil config explain`.
#[derive(Debug, Clone)]
pub struct MenuDef {
    pub name: String,
    pub title: String,
    pub parent: Option<String>,
    pub visible_when: Option<Expr>,
    pub file: PathBuf,
    pub line: u32,
}

#[derive(Debug, Clone)]
pub struct Graph {
    pub options: BTreeMap<String, OptionDef>,
    pub choices: BTreeMap<String, ChoiceDef>,
    pub menus: BTreeMap<String, MenuDef>,
    /// All option names (members included) in file order — the stable
    /// declaration order (files sorted by name, tables in file order).
    pub decl_order: Vec<String>,
}

impl Graph {
    /// Override-file key for a choice (`arch` → `ARCH`).
    pub fn choice_key(name: &str) -> String {
        name.to_uppercase()
    }

    /// Find the choice a given override key addresses, if any.
    pub fn choice_for_key(&self, key: &str) -> Option<&ChoiceDef> {
        self.choices
            .values()
            .find(|c| Self::choice_key(&c.name) == key)
    }
}

fn type_err(file: &Path, line: u32, msg: impl Into<String>) -> Diagnostic {
    Diagnostic::new(Code::EType, msg).at(file, line)
}

/// Parse the `default = …` value for an option of type `ty`. A string value
/// on a non-string option is a guard-chain expression; on a string option it
/// is a guard chain only when it starts with a quoted literal (`"…" if …`),
/// otherwise a plain literal.
fn parse_default_value(
    ty: Type,
    value: &Value,
    file: &Path,
    line: u32,
) -> Result<DefaultSpec, Diagnostic> {
    match (ty, value) {
        (Type::Bool, Value::Bool(_)) | (Type::Int, Value::Int(_)) => {
            Ok(DefaultSpec::literal(value.clone()))
        }
        (Type::IntList, Value::IntList(_)) => Ok(DefaultSpec::literal(value.clone())),
        (Type::IntList, Value::Array(items)) if items.is_empty() => {
            Ok(DefaultSpec::literal(Value::IntList(Vec::new())))
        }
        (Type::Bool | Type::Int, Value::Str(src)) => {
            expr::parse_default(src).map_err(|d| d.at(file, line))
        }
        (Type::Str, Value::Str(s)) => {
            if s.trim_start().starts_with('"') {
                expr::parse_default(s).map_err(|d| d.at(file, line))
            } else {
                Ok(DefaultSpec::literal(Value::Str(s.clone())))
            }
        }
        _ => Err(type_err(
            file,
            line,
            format!(
                "default value {:?} does not fit a {} option",
                value,
                ty.name()
            ),
        )),
    }
}

fn parse_depends(value: &Value, file: &Path, line: u32) -> Result<Expr, Diagnostic> {
    match value {
        Value::Str(src) => expr::parse_expr(src).map_err(|d| d.at(file, line)),
        _ => Err(type_err(
            file,
            line,
            "depends_on must be an expression string",
        )),
    }
}

fn expect_str(value: &Value, key: &str, file: &Path, line: u32) -> Result<String, Diagnostic> {
    match value {
        Value::Str(s) => Ok(s.clone()),
        _ => Err(type_err(file, line, format!("`{}` must be a string", key))),
    }
}

fn expect_bool(value: &Value, key: &str, file: &Path, line: u32) -> Result<bool, Diagnostic> {
    match value {
        Value::Bool(b) => Ok(*b),
        _ => Err(type_err(file, line, format!("`{}` must be a bool", key))),
    }
}

fn expect_int(value: &Value, key: &str, file: &Path, line: u32) -> Result<i64, Diagnostic> {
    match value {
        Value::Int(n) => Ok(*n),
        _ => Err(type_err(file, line, format!("`{}` must be an int", key))),
    }
}

fn parse_scope(value: &Value, file: &Path, line: u32) -> Result<Scope, Diagnostic> {
    match value {
        Value::Str(s) if s == "kernel" => Ok(Scope::Kernel),
        Value::Str(s) if s == "userland" => Ok(Scope::Userland),
        _ => Err(type_err(
            file,
            line,
            "scope must be \"kernel\" or \"userland\"",
        )),
    }
}

/// Build an OptionDef from a table body. `member_of` is Some for
/// `[choice.X.option.M]` tables.
fn build_option(
    name: &str,
    table: &Table,
    member_of: Option<&str>,
    file: &Path,
) -> Result<OptionDef, Diagnostic> {
    let mut def = OptionDef {
        name: name.to_string(),
        ty: if member_of.is_some() {
            Type::ChoiceMember
        } else {
            Type::Bool
        },
        default: None,
        depends_on: None,
        select: Vec::new(),
        computed: false,
        min: None,
        max: None,
        title: None,
        help: None,
        rust_cfg: None,
        rust_cfg_each_prefix: None,
        scope: Scope::None,
        parent_choice: member_of.map(|s| s.to_string()),
        value_label: None,
        menu: None,
        file: file.to_path_buf(),
        line: table.line,
    };

    // `type` must be interpreted before `default`; scan for it first.
    if member_of.is_none() {
        for e in &table.entries {
            if e.key == "type" {
                let t = expect_str(&e.value, "type", file, e.line)?;
                def.ty = match t.as_str() {
                    "bool" => Type::Bool,
                    // No module state today: tristate resolves to bool.
                    "tristate" => Type::Bool,
                    "string" => Type::Str,
                    "int" => Type::Int,
                    "int-list" => Type::IntList,
                    other => {
                        return Err(type_err(file, e.line, format!("unknown type `{}`", other)));
                    }
                };
            }
        }
    }

    for e in &table.entries {
        let line = e.line;
        match e.key.as_str() {
            "type" => {
                if member_of.is_some() {
                    return Err(type_err(file, line, "choice members do not take `type`"));
                }
            }
            "default" => {
                if member_of.is_some() {
                    return Err(type_err(
                        file,
                        line,
                        "choice members do not take `default`; set the choice's `default`",
                    ));
                }
                def.default = Some(parse_default_value(def.ty, &e.value, file, line)?);
            }
            "depends_on" => def.depends_on = Some(parse_depends(&e.value, file, line)?),
            "select" => match &e.value {
                Value::Array(items) => def.select = items.clone(),
                _ => return Err(type_err(file, line, "select must be an array of names")),
            },
            "computed" => def.computed = expect_bool(&e.value, "computed", file, line)?,
            "min" => def.min = Some(expect_int(&e.value, "min", file, line)?),
            "max" => def.max = Some(expect_int(&e.value, "max", file, line)?),
            "title" => def.title = Some(expect_str(&e.value, "title", file, line)?),
            "help" => def.help = Some(expect_str(&e.value, "help", file, line)?),
            "rust_cfg" => def.rust_cfg = Some(expect_str(&e.value, "rust_cfg", file, line)?),
            "rust_cfg_each_prefix" => {
                def.rust_cfg_each_prefix =
                    Some(expect_str(&e.value, "rust_cfg_each_prefix", file, line)?)
            }
            "scope" => def.scope = parse_scope(&e.value, file, line)?,
            "value" => {
                if member_of.is_none() {
                    return Err(type_err(
                        file,
                        line,
                        "`value` is only valid on choice members",
                    ));
                }
                def.value_label = Some(expect_str(&e.value, "value", file, line)?);
            }
            "menu" => def.menu = Some(expect_str(&e.value, "menu", file, line)?),
            other => {
                return Err(type_err(
                    file,
                    line,
                    format!("unknown option key `{}`", other),
                ));
            }
        }
    }

    // Members default their external value to the lowercased tail of their
    // name relative to the choice (ARCH_X86_64 in choice `arch` → "x86_64").
    if let Some(choice) = member_of {
        if def.value_label.is_none() {
            let prefix = format!("{}_", Graph::choice_key(choice));
            let tail = def.name.strip_prefix(&prefix).unwrap_or(&def.name);
            def.value_label = Some(tail.to_lowercase());
        }
    }
    Ok(def)
}

fn build_choice(name: &str, table: &Table, file: &Path) -> Result<ChoiceDef, Diagnostic> {
    let mut def = ChoiceDef {
        name: name.to_string(),
        title: None,
        default_member: String::new(),
        members: Vec::new(),
        depends_on: None,
        help: None,
        menu: None,
        file: file.to_path_buf(),
        line: table.line,
    };
    for e in &table.entries {
        let line = e.line;
        match e.key.as_str() {
            "title" => def.title = Some(expect_str(&e.value, "title", file, line)?),
            "default" => def.default_member = expect_str(&e.value, "default", file, line)?,
            "depends_on" => def.depends_on = Some(parse_depends(&e.value, file, line)?),
            "help" => def.help = Some(expect_str(&e.value, "help", file, line)?),
            "menu" => def.menu = Some(expect_str(&e.value, "menu", file, line)?),
            other => {
                return Err(type_err(
                    file,
                    line,
                    format!("unknown choice key `{}`", other),
                ));
            }
        }
    }
    if def.default_member.is_empty() {
        return Err(Diagnostic::new(
            Code::EType,
            format!("choice `{}` needs a `default` member", name),
        )
        .at(file, table.line));
    }
    Ok(def)
}

fn build_menu(name: &str, table: &Table, file: &Path) -> Result<MenuDef, Diagnostic> {
    let mut def = MenuDef {
        name: name.to_string(),
        title: String::new(),
        parent: None,
        visible_when: None,
        file: file.to_path_buf(),
        line: table.line,
    };
    for e in &table.entries {
        let line = e.line;
        match e.key.as_str() {
            "title" => def.title = expect_str(&e.value, "title", file, line)?,
            "parent" => def.parent = Some(expect_str(&e.value, "parent", file, line)?),
            "visible_when" => def.visible_when = Some(parse_depends(&e.value, file, line)?),
            other => {
                return Err(type_err(
                    file,
                    line,
                    format!("unknown menu key `{}`", other),
                ));
            }
        }
    }
    if def.title.is_empty() {
        return Err(
            Diagnostic::new(Code::EType, format!("menu `{}` needs a `title`", name))
                .at(file, table.line),
        );
    }
    Ok(def)
}

/// Assemble a graph from parsed documents (file order = precedence order for
/// decl_order). Errors accumulate; a non-empty error list fails the load.
pub fn from_docs(docs: &[Doc]) -> Result<Graph, Vec<Diagnostic>> {
    let mut errors: Vec<Diagnostic> = Vec::new();
    let mut graph = Graph {
        options: BTreeMap::new(),
        choices: BTreeMap::new(),
        menus: BTreeMap::new(),
        decl_order: Vec::new(),
    };

    for doc in docs {
        let file = doc.file.as_path();
        for table in &doc.tables {
            if table.path.is_empty() {
                if !table.entries.is_empty() {
                    errors.push(
                        Diagnostic::new(
                            Code::EParse,
                            "graph files take only [option.*] / [choice.*] tables",
                        )
                        .at(file, table.entries[0].line),
                    );
                }
                continue;
            }
            let parts: Vec<&str> = table.path.iter().map(|s| s.as_str()).collect();
            let result: Result<(), Diagnostic> = match parts.as_slice() {
                ["option", name] => match build_option(name, table, None, file) {
                    Ok(def) => {
                        if graph.options.contains_key(*name) {
                            Err(Diagnostic::new(
                                Code::EDup,
                                format!("option `{}` defined more than once", name),
                            )
                            .at(file, table.line))
                        } else {
                            graph.decl_order.push(name.to_string());
                            graph.options.insert(name.to_string(), def);
                            Ok(())
                        }
                    }
                    Err(d) => Err(d),
                },
                ["menu", name] => match build_menu(name, table, file) {
                    Ok(def) => {
                        if graph.menus.contains_key(*name) {
                            Err(Diagnostic::new(
                                Code::EDup,
                                format!("menu `{}` defined more than once", name),
                            )
                            .at(file, table.line))
                        } else {
                            graph.menus.insert(name.to_string(), def);
                            Ok(())
                        }
                    }
                    Err(d) => Err(d),
                },
                ["choice", name] => match build_choice(name, table, file) {
                    Ok(def) => {
                        if graph.choices.contains_key(*name) {
                            Err(Diagnostic::new(
                                Code::EDup,
                                format!("choice `{}` defined more than once", name),
                            )
                            .at(file, table.line))
                        } else {
                            graph.choices.insert(name.to_string(), def);
                            Ok(())
                        }
                    }
                    Err(d) => Err(d),
                },
                ["choice", choice, "option", member] => {
                    if !graph.choices.contains_key(*choice) {
                        Err(Diagnostic::new(
                            Code::EUnknownRef,
                            format!(
                                "member `{}` declared before its choice `{}`",
                                member, choice
                            ),
                        )
                        .at(file, table.line))
                    } else {
                        match build_option(member, table, Some(choice), file) {
                            Ok(def) => {
                                if graph.options.contains_key(*member) {
                                    Err(Diagnostic::new(
                                        Code::EDup,
                                        format!("option `{}` defined more than once", member),
                                    )
                                    .at(file, table.line))
                                } else {
                                    graph.decl_order.push(member.to_string());
                                    graph.options.insert(member.to_string(), def);
                                    graph
                                        .choices
                                        .get_mut(*choice)
                                        .expect("checked above")
                                        .members
                                        .push(member.to_string());
                                    Ok(())
                                }
                            }
                            Err(d) => Err(d),
                        }
                    }
                }
                _ => Err(Diagnostic::new(
                    Code::EParse,
                    format!("unrecognized table `[{}]`", table.path.join(".")),
                )
                .at(file, table.line)),
            };
            if let Err(d) = result {
                errors.push(d);
            }
        }
    }

    // A choice's override key is its uppercased name; that key must not
    // collide with an option name.
    for choice in graph.choices.values() {
        let key = Graph::choice_key(&choice.name);
        if graph.options.contains_key(&key) {
            errors.push(
                Diagnostic::new(
                    Code::EDup,
                    format!(
                        "choice `{}` collides with option `{}` (override keys are ambiguous)",
                        choice.name, key
                    ),
                )
                .at(&choice.file, choice.line),
            );
        }
    }

    if errors.is_empty() {
        Ok(graph)
    } else {
        Err(errors)
    }
}

/// Load and assemble all `*.toml` graph files under `dir`, sorted by name.
pub fn load(dir: &Path) -> Result<Graph, Vec<Diagnostic>> {
    let mut files: Vec<PathBuf> = match std::fs::read_dir(dir) {
        Ok(entries) => entries
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "toml"))
            .collect(),
        Err(e) => {
            return Err(vec![Diagnostic::new(
                Code::EParse,
                format!("cannot read graph directory {}: {}", dir.display(), e),
            )]);
        }
    };
    files.sort();
    if files.is_empty() {
        return Err(vec![Diagnostic::new(
            Code::EParse,
            format!("no *.toml graph files in {}", dir.display()),
        )]);
    }
    let mut docs = Vec::new();
    for path in &files {
        let src = match std::fs::read_to_string(path) {
            Ok(s) => s,
            Err(e) => {
                return Err(vec![Diagnostic::new(
                    Code::EParse,
                    format!("cannot read {}: {}", path.display(), e),
                )]);
            }
        };
        match toml::parse(path, &src) {
            Ok(doc) => docs.push(doc),
            Err(d) => return Err(vec![d]),
        }
    }
    from_docs(&docs)
}

/// Names an option or choice a definition references.
fn referenced_names(def: &OptionDef) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(e) = &def.depends_on {
        e.atoms(&mut out);
    }
    if let Some(d) = &def.default {
        d.atoms(&mut out);
    }
    out
}

/// Structural validation: reference existence, type sanity, select cycles,
/// dependency cycles. Returns all diagnostics found (errors only).
pub fn validate(graph: &Graph) -> Vec<Diagnostic> {
    let mut errors = Vec::new();

    let known = |name: &str| -> bool {
        graph.options.contains_key(name) || graph.choices.contains_key(name)
    };

    for def in graph.options.values() {
        // Reference existence.
        for name in referenced_names(def) {
            if !known(&name) {
                errors.push(
                    Diagnostic::new(
                        Code::EUnknownRef,
                        format!("`{}` references unknown `{}`", def.name, name),
                    )
                    .at(&def.file, def.line),
                );
            }
        }
        for target in &def.select {
            match graph.options.get(target) {
                None => errors.push(
                    Diagnostic::new(
                        Code::EUnknownRef,
                        format!("`{}` selects unknown `{}`", def.name, target),
                    )
                    .at(&def.file, def.line),
                ),
                Some(t) if !matches!(t.ty, Type::Bool | Type::ChoiceMember) => errors.push(
                    Diagnostic::new(
                        Code::EType,
                        format!(
                            "`{}` selects `{}`, which is {} (only bool/choice-member)",
                            def.name,
                            target,
                            t.ty.name()
                        ),
                    )
                    .at(&def.file, def.line),
                ),
                Some(_) => {}
            }
        }
        if !def.select.is_empty() && !matches!(def.ty, Type::Bool | Type::ChoiceMember) {
            errors.push(
                Diagnostic::new(
                    Code::EType,
                    format!("`{}` has `select` but is {}", def.name, def.ty.name()),
                )
                .at(&def.file, def.line),
            );
        }

        // Type sanity.
        if (def.min.is_some() || def.max.is_some()) && !matches!(def.ty, Type::Int | Type::IntList)
        {
            errors.push(
                Diagnostic::new(
                    Code::EType,
                    format!(
                        "`{}`: min/max are only valid on int and int-list options",
                        def.name
                    ),
                )
                .at(&def.file, def.line),
            );
        }
        if let (Some(min), Some(max)) = (def.min, def.max) {
            if min > max {
                errors.push(
                    Diagnostic::new(Code::ERange, format!("`{}`: min > max", def.name))
                        .at(&def.file, def.line),
                );
            }
        }
        if def.rust_cfg_each_prefix.is_some() && def.ty != Type::Str {
            errors.push(
                Diagnostic::new(
                    Code::EType,
                    format!(
                        "`{}`: rust_cfg_each_prefix is only valid on string options",
                        def.name
                    ),
                )
                .at(&def.file, def.line),
            );
        }
        if let Some(spec) = &def.default {
            for (value, _) in &spec.arms {
                let ok = matches!(
                    (def.ty, value),
                    (Type::Bool, Value::Bool(_))
                        | (Type::Int, Value::Int(_))
                        | (Type::IntList, Value::IntList(_))
                        | (Type::Str, Value::Str(_))
                );
                if !ok {
                    errors.push(
                        Diagnostic::new(
                            Code::EType,
                            format!(
                                "`{}`: default arm {:?} does not fit type {}",
                                def.name,
                                value,
                                def.ty.name()
                            ),
                        )
                        .at(&def.file, def.line),
                    );
                }
                let elements: &[i64] = match value {
                    Value::Int(n) => std::slice::from_ref(n),
                    Value::IntList(items) => items,
                    _ => &[],
                };
                for n in elements {
                    if def.out_of_range(*n) {
                        errors.push(
                            Diagnostic::new(
                                Code::ERange,
                                format!("`{}`: default {} outside [min, max]", def.name, n),
                            )
                            .at(&def.file, def.line),
                        );
                    }
                }
            }
        }
    }

    for choice in graph.choices.values() {
        if choice.members.is_empty() {
            errors.push(
                Diagnostic::new(
                    Code::EType,
                    format!("choice `{}` has no members", choice.name),
                )
                .at(&choice.file, choice.line),
            );
            continue;
        }
        if !choice.members.contains(&choice.default_member) {
            errors.push(
                Diagnostic::new(
                    Code::EUnknownRef,
                    format!(
                        "choice `{}` default `{}` is not one of its members",
                        choice.name, choice.default_member
                    ),
                )
                .at(&choice.file, choice.line),
            );
        }
        if let Some(e) = &choice.depends_on {
            let mut names = Vec::new();
            e.atoms(&mut names);
            for name in names {
                if !known(&name) {
                    errors.push(
                        Diagnostic::new(
                            Code::EUnknownRef,
                            format!("choice `{}` references unknown `{}`", choice.name, name),
                        )
                        .at(&choice.file, choice.line),
                    );
                }
            }
        }
    }

    if !errors.is_empty() {
        // Cycle detection below assumes resolvable references.
        return errors;
    }

    // Select cycles (DFS over select edges).
    if let Some(cycle) = find_cycle(graph, |name| {
        graph
            .options
            .get(name)
            .map(|d| d.select.clone())
            .unwrap_or_default()
    }) {
        errors.push(
            Diagnostic::new(Code::ESelectCycle, "select edges form a cycle").with_chain(cycle),
        );
    }

    // Dependency cycles over the reference graph (depends_on + guard atoms;
    // members depend on their choice; choices on their depends_on atoms).
    if let Some(cycle) = find_cycle(graph, |name| {
        let mut out = Vec::new();
        if let Some(def) = graph.options.get(name) {
            out.extend(referenced_names(def));
            if let Some(c) = &def.parent_choice {
                out.push(c.clone());
            }
        } else if let Some(choice) = graph.choices.get(name) {
            if let Some(e) = &choice.depends_on {
                e.atoms(&mut out);
            }
        }
        out
    }) {
        errors.push(
            Diagnostic::new(Code::EDepCycle, "dependency references form a cycle")
                .with_chain(cycle),
        );
    }

    // Menu construct (organization-only): membership, parent, visibility.
    for def in graph.options.values() {
        if let Some(m) = &def.menu {
            if !graph.menus.contains_key(m) {
                errors.push(
                    Diagnostic::new(
                        Code::EUnknownMenu,
                        format!("option `{}` names unknown menu `{}`", def.name, m),
                    )
                    .at(&def.file, def.line),
                );
            }
        }
    }
    for choice in graph.choices.values() {
        if let Some(m) = &choice.menu {
            if !graph.menus.contains_key(m) {
                errors.push(
                    Diagnostic::new(
                        Code::EUnknownMenu,
                        format!("choice `{}` names unknown menu `{}`", choice.name, m),
                    )
                    .at(&choice.file, choice.line),
                );
            }
        }
    }
    for menu in graph.menus.values() {
        if let Some(p) = &menu.parent {
            if !graph.menus.contains_key(p) {
                errors.push(
                    Diagnostic::new(
                        Code::EUnknownMenu,
                        format!("menu `{}` names unknown parent `{}`", menu.name, p),
                    )
                    .at(&menu.file, menu.line),
                );
            }
        }
        if let Some(e) = &menu.visible_when {
            let mut atoms = Vec::new();
            e.atoms(&mut atoms);
            for name in atoms {
                if !known(&name) {
                    errors.push(
                        Diagnostic::new(
                            Code::EUnknownRef,
                            format!(
                                "menu `{}` visible_when references unknown `{}`",
                                menu.name, name
                            ),
                        )
                        .at(&menu.file, menu.line),
                    );
                }
            }
        }
        // Parent-chain cycle from this menu.
        let mut seen: Vec<&str> = Vec::new();
        let mut cur = Some(menu.name.as_str());
        while let Some(n) = cur {
            if seen.contains(&n) {
                errors.push(
                    Diagnostic::new(
                        Code::EMenuCycle,
                        format!("menu `{}` is in a parent-chain cycle", menu.name),
                    )
                    .at(&menu.file, menu.line),
                );
                break;
            }
            seen.push(n);
            cur = graph.menus.get(n).and_then(|m| m.parent.as_deref());
        }
    }

    errors
}

/// Iterative DFS cycle finder over `edges(node)`. Returns the cycle path
/// (first repeated node closes it) or None.
fn find_cycle(graph: &Graph, edges: impl Fn(&str) -> Vec<String>) -> Option<Vec<String>> {
    #[derive(Clone, Copy, PartialEq)]
    enum Mark {
        InProgress,
        Done,
    }
    let mut marks: BTreeMap<String, Mark> = BTreeMap::new();

    fn visit(
        node: &str,
        edges: &impl Fn(&str) -> Vec<String>,
        marks: &mut BTreeMap<String, Mark>,
        stack: &mut Vec<String>,
    ) -> Option<Vec<String>> {
        match marks.get(node) {
            Some(Mark::Done) => return None,
            Some(Mark::InProgress) => {
                let start = stack.iter().position(|n| n == node).unwrap_or(0);
                let mut cycle: Vec<String> = stack[start..].to_vec();
                cycle.push(node.to_string());
                return Some(cycle);
            }
            None => {}
        }
        marks.insert(node.to_string(), Mark::InProgress);
        stack.push(node.to_string());
        for next in edges(node) {
            if let Some(cycle) = visit(&next, edges, marks, stack) {
                return Some(cycle);
            }
        }
        stack.pop();
        marks.insert(node.to_string(), Mark::Done);
        None
    }

    let mut roots: Vec<&String> = graph.options.keys().collect();
    let choice_names: Vec<&String> = graph.choices.keys().collect();
    roots.extend(choice_names);
    for root in roots {
        let mut stack = Vec::new();
        if let Some(cycle) = visit(root, &edges, &mut marks, &mut stack) {
            return Some(cycle);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    pub(crate) fn graph_from(src: &str) -> Result<Graph, Vec<Diagnostic>> {
        let doc = toml::parse(Path::new("g.toml"), src).map_err(|d| vec![d])?;
        from_docs(&[doc])
    }

    #[test]
    fn loads_options_choices_members() {
        let g = graph_from(
            r#"
[option.NET]
type = "bool"
default = true
select = ["NETDRV"]
depends_on = "BUILD_USERLAND"

[option.NETDRV]
type = "bool"
default = false

[option.BUILD_USERLAND]
type = "bool"
default = true

[choice.arch]
default = "ARCH_X86_64"
  [choice.arch.option.ARCH_X86_64]
  [choice.arch.option.ARCH_AARCH64]
"#,
        )
        .unwrap();
        assert_eq!(
            g.choices["arch"].members,
            vec!["ARCH_X86_64", "ARCH_AARCH64"]
        );
        assert_eq!(
            g.options["ARCH_X86_64"].value_label.as_deref(),
            Some("x86_64")
        );
        assert_eq!(g.options["NET"].select, vec!["NETDRV"]);
        assert_eq!(
            g.decl_order,
            vec![
                "NET",
                "NETDRV",
                "BUILD_USERLAND",
                "ARCH_X86_64",
                "ARCH_AARCH64"
            ]
        );
        assert!(validate(&g).is_empty());
    }

    #[test]
    fn tristate_resolves_to_bool() {
        let g = graph_from("[option.X]\ntype = \"tristate\"\ndefault = true\n").unwrap();
        assert_eq!(g.options["X"].ty, Type::Bool);
    }

    #[test]
    fn guard_default_parses() {
        let g = graph_from(
            "[option.DEBUG_SERIAL]\ntype = \"bool\"\ndefault = true\n\
             [option.NO_SERIAL_OUTPUT]\ntype = \"bool\"\ncomputed = true\n\
             default = \"true if !DEBUG_SERIAL else false\"\nrust_cfg = \"no_serial_output\"\nscope = \"kernel\"\n",
        )
        .unwrap();
        assert_eq!(
            g.options["NO_SERIAL_OUTPUT"]
                .default
                .as_ref()
                .unwrap()
                .arms
                .len(),
            2
        );
        assert!(validate(&g).is_empty());
    }

    #[test]
    fn unknown_refs_and_bad_types() {
        let g = graph_from("[option.A]\ntype = \"bool\"\ndepends_on = \"MISSING\"\n").unwrap();
        let errs = validate(&g);
        assert!(errs.iter().any(|d| d.code == Code::EUnknownRef));

        let g = graph_from("[option.A]\ntype = \"string\"\nmin = 1\n").unwrap();
        assert!(validate(&g).iter().any(|d| d.code == Code::EType));

        let g = graph_from("[option.A]\ntype = \"int\"\nmin = 1\nmax = 4\ndefault = 9\n").unwrap();
        assert!(validate(&g).iter().any(|d| d.code == Code::ERange));
    }

    #[test]
    fn select_cycle_detected() {
        let g = graph_from(
            "[option.A]\ntype = \"bool\"\nselect = [\"B\"]\n\
             [option.B]\ntype = \"bool\"\nselect = [\"A\"]\n",
        )
        .unwrap();
        let errs = validate(&g);
        let cyc = errs.iter().find(|d| d.code == Code::ESelectCycle).unwrap();
        assert!(cyc.chain.len() >= 3);
    }

    #[test]
    fn dep_cycle_detected() {
        let g = graph_from(
            "[option.A]\ntype = \"bool\"\ndepends_on = \"B\"\n\
             [option.B]\ntype = \"bool\"\ndepends_on = \"A\"\n",
        )
        .unwrap();
        assert!(validate(&g).iter().any(|d| d.code == Code::EDepCycle));
    }

    #[test]
    fn duplicate_and_collision() {
        let errs = graph_from("[option.A]\n[option.A]\n").unwrap_err();
        // toml-level duplicate table is E_PARSE; cross-file duplicates are
        // E_DUP — both fail the load.
        assert!(!errs.is_empty());

        let errs = graph_from("[option.ARCH]\ntype = \"bool\"\n[choice.arch]\ndefault = \"M\"\n  [choice.arch.option.M]\n")
            .unwrap_err();
        assert!(errs.iter().any(|d| d.code == Code::EDup));
    }

    #[test]
    fn member_before_choice_rejected() {
        let errs = graph_from("[choice.c.option.M]\n").unwrap_err();
        assert!(errs.iter().any(|d| d.code == Code::EUnknownRef));
    }
}
