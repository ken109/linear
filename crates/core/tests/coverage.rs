//! Coverage of Linear's schema.
//!
//! `coverage.toml` classifies every field of the Query, Mutation and
//! Subscription roots, and of the core entities, as implemented or
//! unsupported (with a reason). These tests fail when the schema gains a field
//! nobody has classified, when the manifest names a field that is gone, and
//! when the manifest disagrees with what the code actually selects.
//!
//! The checks are plain functions over (schema, manifest, source) text, so the
//! last test group proves they really do fail on bad input.

use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

const SCHEMA: &str = include_str!("../../../schema/linear.graphql");
const MANIFEST: &str = include_str!("../coverage.toml");

/// Root types: classified by whether a typed query or mutation selects the field.
const ROOTS: &[&str] = &["Query", "Mutation", "Subscription"];
/// Core entities: classified by whether their fixed fragment (and so `--json`) includes the field.
const ENTITIES: &[&str] = &[
    "Issue",
    "Project",
    "ProjectMilestone",
    "ProjectUpdate",
    "Initiative",
    "User",
    "Template",
    "Team",
    "IssueLabel",
    "Comment",
    "Attachment",
];

fn tracked() -> Vec<&'static str> {
    ROOTS.iter().chain(ENTITIES).copied().collect()
}

// ------------------------------------------------------------------ manifest

#[derive(Debug, Deserialize)]
struct Manifest {
    entry: Vec<Entry>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    r#type: String,
    status: Status,
    fields: Vec<String>,
    #[serde(default)]
    via: Vec<String>,
    // Free text; documents which CLI commands expose the fields.
    #[serde(default)]
    #[allow(dead_code)]
    commands: Vec<String>,
    reason: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Status {
    Implemented,
    Unsupported,
}

fn parse_manifest(text: &str) -> Manifest {
    toml_edit::de::from_str(text).unwrap_or_else(|e| panic!("coverage.toml does not parse: {e}"))
}

// ------------------------------------------------------------------ schema

/// The fields of each named object type in an SDL document.
fn schema_fields(sdl: &str, wanted: &[&str]) -> BTreeMap<String, BTreeSet<String>> {
    let doc = cynic_parser::parse_type_system_document(sdl).expect("the SDL parses");
    let mut out = BTreeMap::new();
    for def in doc.definitions() {
        if let cynic_parser::type_system::Definition::Type(
            cynic_parser::type_system::TypeDefinition::Object(object),
        ) = def
        {
            if wanted.contains(&object.name()) {
                out.insert(
                    object.name().to_owned(),
                    object.fields().map(|f| f.name().to_owned()).collect(),
                );
            }
        }
    }
    for name in wanted {
        assert!(out.contains_key(*name), "type {name} is not in the schema");
    }
    out
}

// ------------------------------------------------------------------ code

/// The fields selected by `cynic::QueryFragment` structs, by GraphQL type.
///
/// A deliberately small scan of Rust source: it understands `#[cynic(graphql_type
/// = "...")]`, `#[cynic(rename = "...")]` and one `pub field: Type,` per line,
/// which is how every fragment in this crate is written.
fn fragment_fields(src: &str) -> BTreeMap<String, BTreeSet<String>> {
    let mut out: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut attrs: Vec<&str> = Vec::new();
    let mut current: Option<String> = None;

    for line in src.lines() {
        let t = line.trim();
        if t.starts_with("//") {
            continue;
        }
        if t.starts_with("#[") {
            attrs.push(t);
            continue;
        }
        match &current {
            None => {
                let name = t
                    .strip_prefix("pub struct ")
                    .or_else(|| t.strip_prefix("struct "))
                    .map(ident);
                if let Some(name) = name {
                    if attrs.iter().any(|a| a.contains("QueryFragment")) {
                        let ty = attr_value(&attrs, "graphql_type").unwrap_or(name);
                        out.entry(ty.clone()).or_default();
                        if t.ends_with('{') {
                            current = Some(ty);
                        }
                    }
                }
            }
            Some(ty) => {
                if t.starts_with('}') {
                    current = None;
                } else if let Some(field) = t.strip_prefix("pub ") {
                    let rust_name = ident(field);
                    let name =
                        attr_value(&attrs, "rename").unwrap_or_else(|| camel_case(&rust_name));
                    out.get_mut(ty).expect("registered").insert(name);
                }
            }
        }
        attrs.clear();
    }
    out
}

fn ident(s: &str) -> String {
    s.chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
        .collect()
}

/// The value of `key = "value"` in a list of attribute lines.
fn attr_value(attrs: &[&str], key: &str) -> Option<String> {
    for a in attrs {
        let needle = format!("{key} = \"");
        if let Some(i) = a.find(&needle) {
            let rest = &a[i + needle.len()..];
            return rest.split('"').next().map(str::to_owned);
        }
    }
    None
}

fn camel_case(snake: &str) -> String {
    let mut out = String::new();
    let mut upper = false;
    for c in snake.chars() {
        if c == '_' {
            upper = true;
        } else if upper {
            out.extend(c.to_uppercase());
            upper = false;
        } else {
            out.push(c);
        }
    }
    out
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).expect("read src") {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

struct Sources {
    /// `module name -> source text`, for the `via` check.
    modules: BTreeMap<String, String>,
    /// Fields selected by root-type fragments anywhere in the crate.
    roots: BTreeMap<String, BTreeSet<String>>,
    /// Fields selected by entity fragments in `types.rs`.
    entities: BTreeMap<String, BTreeSet<String>>,
}

fn load_sources() -> Sources {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    rust_files(&src, &mut files);

    let mut modules = BTreeMap::new();
    let mut roots: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for file in &files {
        let text = fs::read_to_string(file).expect("read source");
        for (ty, fields) in fragment_fields(&text) {
            if ROOTS.contains(&ty.as_str()) {
                roots.entry(ty).or_default().extend(fields);
            }
        }
        if let Some(stem) = file.file_stem() {
            modules.insert(stem.to_string_lossy().into_owned(), text);
        }
    }
    let entities = fragment_fields(&modules["types"]);
    Sources {
        modules,
        roots,
        entities,
    }
}

// ------------------------------------------------------------------ checks

/// Every tracked field is classified exactly once, and nothing stale remains.
fn classification_problems(
    schema: &BTreeMap<String, BTreeSet<String>>,
    manifest: &Manifest,
) -> Vec<String> {
    let mut problems = Vec::new();
    let mut seen: BTreeMap<(&str, &str), usize> = BTreeMap::new();

    for e in &manifest.entry {
        let Some(fields) = schema.get(&e.r#type) else {
            problems.push(format!(
                "entry for type {:?}, which is not tracked (tracked: {})",
                e.r#type,
                schema.keys().cloned().collect::<Vec<_>>().join(", ")
            ));
            continue;
        };
        for f in &e.fields {
            if !fields.contains(f) {
                problems.push(format!(
                    "{}.{f} is classified but no longer exists in the schema; remove it",
                    e.r#type
                ));
            }
            *seen.entry((e.r#type.as_str(), f.as_str())).or_default() += 1;
        }
    }
    for ((ty, f), n) in &seen {
        if *n > 1 {
            problems.push(format!("{ty}.{f} is classified {n} times"));
        }
    }

    for (ty, fields) in schema {
        let missing: Vec<&String> = fields
            .iter()
            .filter(|f| !seen.contains_key(&(ty.as_str(), f.as_str())))
            .collect();
        if !missing.is_empty() {
            let list = missing
                .iter()
                .map(|f| format!("\"{f}\""))
                .collect::<Vec<_>>()
                .join(", ");
            problems.push(format!(
                "{} field(s) of {ty} are not classified: {}\n    add to crates/core/coverage.toml:\n    \
                 [[entry]]\n    type = \"{ty}\"\n    status = \"unsupported\"\n    reason = \"...\"\n    fields = [{list}]",
                missing.len(),
                missing.iter().map(|f| f.as_str()).collect::<Vec<_>>().join(", "),
            ));
        }
    }
    problems
}

/// Entries are well formed: reasons for unsupported, an existing `via` for implemented.
fn entry_problems(manifest: &Manifest, modules: &BTreeMap<String, String>) -> Vec<String> {
    let mut problems = Vec::new();
    for e in &manifest.entry {
        let label = format!(
            "{} [{}]",
            e.r#type,
            e.fields.first().map_or("", |f| f.as_str())
        );
        if e.fields.is_empty() {
            problems.push(format!("{label}: an entry with no fields"));
        }
        match e.status {
            Status::Unsupported => {
                if e.reason.as_deref().is_none_or(|r| r.trim().is_empty()) {
                    problems.push(format!("{label}: unsupported needs a reason"));
                }
                if !e.via.is_empty() {
                    problems.push(format!("{label}: unsupported must not have `via`"));
                }
            }
            Status::Implemented => {
                if e.via.is_empty() {
                    problems.push(format!("{label}: implemented needs `via`"));
                }
                for v in &e.via {
                    if !item_exists(modules, v) {
                        problems.push(format!(
                            "{label}: `via` {v:?} does not exist in linear-core"
                        ));
                    }
                }
            }
        }
    }
    problems
}

/// `queries::whoami` -> `pub fn whoami` in queries.rs; `types::Issue` -> `pub struct Issue`.
fn item_exists(modules: &BTreeMap<String, String>, path: &str) -> bool {
    let Some((module, item)) = path.split_once("::") else {
        return false;
    };
    let Some(src) = modules.get(module) else {
        return false;
    };
    src.lines().any(|l| {
        let t = l.trim();
        [t.strip_prefix("pub fn "), t.strip_prefix("pub struct ")]
            .into_iter()
            .flatten()
            .any(|rest| ident(rest) == item)
    })
}

/// "Implemented" means exactly "the code selects it".
fn code_problems(
    manifest: &Manifest,
    roots: &BTreeMap<String, BTreeSet<String>>,
    entities: &BTreeMap<String, BTreeSet<String>>,
) -> Vec<String> {
    let mut implemented: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for e in &manifest.entry {
        let set = implemented.entry(e.r#type.as_str()).or_default();
        if e.status == Status::Implemented {
            set.extend(e.fields.iter().map(String::as_str));
        }
    }

    let mut problems = Vec::new();
    for ty in tracked_in(&implemented) {
        let selected = if ROOTS.contains(&ty) {
            roots.get(ty)
        } else {
            entities.get(ty)
        };
        let selected: BTreeSet<&str> = selected
            .map(|s| s.iter().map(String::as_str).collect())
            .unwrap_or_default();
        let claimed = implemented.get(ty).cloned().unwrap_or_default();
        for f in selected.difference(&claimed) {
            problems.push(format!(
                "{ty}.{f} is selected by the code but is not marked implemented in coverage.toml"
            ));
        }
        for f in claimed.difference(&selected) {
            problems.push(format!(
                "{ty}.{f} is marked implemented but the code does not select it"
            ));
        }
    }
    problems
}

fn tracked_in<'a>(m: &BTreeMap<&'a str, BTreeSet<&'a str>>) -> Vec<&'a str> {
    let mut v: Vec<&str> = m.keys().copied().collect();
    for t in tracked() {
        if !v.contains(&t) {
            v.push(t);
        }
    }
    v
}

fn assert_no_problems(problems: Vec<String>) {
    assert!(
        problems.is_empty(),
        "\n{} problem(s):\n\n{}\n",
        problems.len(),
        problems.join("\n\n")
    );
}

// ------------------------------------------------------------------ the real checks

#[test]
fn every_schema_field_is_classified() {
    let manifest = parse_manifest(MANIFEST);
    let schema = schema_fields(SCHEMA, &tracked());
    assert_no_problems(classification_problems(&schema, &manifest));
}

#[test]
fn entries_are_well_formed() {
    let manifest = parse_manifest(MANIFEST);
    assert_no_problems(entry_problems(&manifest, &load_sources().modules));
}

#[test]
fn the_manifest_matches_what_the_code_selects() {
    let manifest = parse_manifest(MANIFEST);
    let sources = load_sources();
    assert_no_problems(code_problems(&manifest, &sources.roots, &sources.entities));
}

// ------------------------------------------------------------------ the checks catch real mistakes

const TINY_SDL: &str = r#"
type Query { viewer: User  issue(id: String!): Issue  extra: String }
type Issue { id: ID!  title: String! }
type User { id: ID! }
"#;

fn tiny_schema() -> BTreeMap<String, BTreeSet<String>> {
    schema_fields(TINY_SDL, &["Query", "Issue"])
}

fn problems_for(manifest: &str) -> Vec<String> {
    classification_problems(&tiny_schema(), &parse_manifest(manifest))
}

const TINY_OK: &str = r#"
[[entry]]
type = "Query"
status = "implemented"
via = ["queries::whoami"]
fields = ["viewer", "issue"]

[[entry]]
type = "Query"
status = "unsupported"
reason = "unused"
fields = ["extra"]

[[entry]]
type = "Issue"
status = "implemented"
via = ["types::Issue"]
fields = ["id", "title"]
"#;

#[test]
fn a_complete_manifest_passes() {
    assert!(problems_for(TINY_OK).is_empty());
}

#[test]
fn an_unclassified_field_fails_and_is_named() {
    let without_extra = TINY_OK.replace(
        "[[entry]]\ntype = \"Query\"\nstatus = \"unsupported\"\nreason = \"unused\"\nfields = [\"extra\"]\n",
        "",
    );
    let problems = problems_for(&without_extra);
    assert_eq!(problems.len(), 1, "{problems:?}");
    assert!(problems[0].contains("Query") && problems[0].contains("extra"));
}

#[test]
fn a_field_missing_from_a_core_entity_fails() {
    let problems = problems_for(&TINY_OK.replace("[\"id\", \"title\"]", "[\"id\"]"));
    assert_eq!(problems.len(), 1, "{problems:?}");
    assert!(problems[0].contains("Issue") && problems[0].contains("title"));
}

#[test]
fn stale_duplicate_and_untracked_entries_fail() {
    let stale = format!(
        "{TINY_OK}\n[[entry]]\ntype = \"Issue\"\nstatus = \"unsupported\"\nreason = \"x\"\nfields = [\"gone\"]\n"
    );
    assert!(problems_for(&stale)
        .iter()
        .any(|p| p.contains("Issue.gone") && p.contains("no longer exists")));

    let dup = format!(
        "{TINY_OK}\n[[entry]]\ntype = \"Issue\"\nstatus = \"unsupported\"\nreason = \"x\"\nfields = [\"id\"]\n"
    );
    assert!(problems_for(&dup)
        .iter()
        .any(|p| p.contains("Issue.id is classified 2 times")));

    let untracked = format!(
        "{TINY_OK}\n[[entry]]\ntype = \"User\"\nstatus = \"unsupported\"\nreason = \"x\"\nfields = [\"id\"]\n"
    );
    assert!(problems_for(&untracked)
        .iter()
        .any(|p| p.contains("not tracked")));
}

#[test]
fn entries_need_reasons_and_real_items() {
    let modules: BTreeMap<String, String> = [(
        "queries".to_owned(),
        "pub fn whoami() {}\npub struct Whoami {}".to_owned(),
    )]
    .into();
    let ok = parse_manifest(TINY_OK);
    // `types::Issue` does not exist in these modules, `queries::whoami` does.
    let problems = entry_problems(&ok, &modules);
    assert_eq!(problems.len(), 1, "{problems:?}");
    assert!(problems[0].contains("types::Issue"));

    let no_reason = parse_manifest(
        "[[entry]]\ntype = \"Query\"\nstatus = \"unsupported\"\nreason = \" \"\nfields = [\"extra\"]\n",
    );
    assert!(entry_problems(&no_reason, &modules)[0].contains("needs a reason"));

    let no_via = parse_manifest(
        "[[entry]]\ntype = \"Query\"\nstatus = \"implemented\"\nfields = [\"viewer\"]\n",
    );
    assert!(entry_problems(&no_via, &modules)[0].contains("needs `via`"));
}

#[test]
fn the_manifest_must_agree_with_the_code() {
    let manifest = parse_manifest(TINY_OK);
    let code = |q: &[&str], i: &[&str]| {
        let set = |s: &[&str]| s.iter().map(|f| (*f).to_owned()).collect::<BTreeSet<_>>();
        (
            BTreeMap::from([("Query".to_owned(), set(q))]),
            BTreeMap::from([("Issue".to_owned(), set(i))]),
        )
    };

    let (roots, entities) = code(&["viewer", "issue"], &["id", "title"]);
    assert!(code_problems(&manifest, &roots, &entities).is_empty());

    // The code selects something the manifest calls unsupported.
    let (roots, entities) = code(&["viewer", "issue", "extra"], &["id", "title"]);
    let p = code_problems(&manifest, &roots, &entities);
    assert!(p.len() == 1 && p[0].contains("Query.extra"), "{p:?}");

    // The manifest claims a field the code does not select.
    let (roots, entities) = code(&["viewer", "issue"], &["id"]);
    let p = code_problems(&manifest, &roots, &entities);
    assert!(p.len() == 1 && p[0].contains("Issue.title"), "{p:?}");
}

#[test]
fn fragment_scanning_reads_names_renames_and_graphql_types() {
    let src = r#"
/// Doc.
#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "ProjectUpdate")]
pub struct StatusUpdate {
    pub id: cynic::Id,
    #[cynic(rename = "type")]
    pub type_: String,
    #[arguments(first: 50)]
    pub created_at: DateTime<Utc>,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct User {
    pub display_name: String,
}

#[derive(Debug)]
pub struct NotAFragment {
    pub ignored: String,
}
"#;
    let f = fragment_fields(src);
    assert_eq!(
        f["ProjectUpdate"],
        BTreeSet::from(["id", "type", "createdAt"].map(String::from))
    );
    assert_eq!(f["User"], BTreeSet::from(["displayName".to_owned()]));
    assert!(!f.contains_key("NotAFragment"));
}
