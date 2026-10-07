//! `linear usage` and `linear <group> usage`: how to use the CLI, for an AI agent.
//!
//! Everything about the commands (their names, what they do, their flags) is read from the
//! clap command definition in `cli.rs`, so it cannot drift from what the binary accepts: a
//! test walks the whole tree and fails when a command or a flag is missing from the text.
//! What clap cannot know is a small hand-written layer, kept in this file: which commands
//! only read, the meaning of each exit code, the safety model, and a note or two per group.
//! Needs no configuration, credentials or network.

use crate::cli::{Cli, Command as Top};
use crate::commands::{cycle, webhook};
use crate::error::{CliError, Result};
use crate::output::Output;
use clap::{Arg, ArgAction, CommandFactory};
use linear_core::ErrorCode;
use serde::Serialize;

/// What was asked for: the overview, or one group in detail.
pub enum Scope {
    All,
    Group(&'static str),
}

/// The `usage` command in `command`, if that is what was typed.
pub fn requested(command: &Top) -> Option<Scope> {
    use crate::cli::WorkspaceCommand as Workspace;
    use crate::commands::{cache, comment, document, file, initiative, issue, label, milestone};
    use crate::commands::{project, team, template, user};
    Some(match command {
        Top::Usage => Scope::All,
        Top::Workspace(Workspace::Usage) => Scope::Group("workspace"),
        Top::Issue(issue::IssueCommand::Usage) => Scope::Group("issue"),
        Top::Comment(comment::CommentCommand::Usage) => Scope::Group("comment"),
        Top::Project(project::ProjectCommand::Usage) => Scope::Group("project"),
        Top::Milestone(milestone::MilestoneCommand::Usage) => Scope::Group("milestone"),
        Top::Initiative(initiative::InitiativeCommand::Usage) => Scope::Group("initiative"),
        Top::File(file::FileCommand::Usage) => Scope::Group("file"),
        Top::Template(template::TemplateCommand::Usage) => Scope::Group("template"),
        Top::Label(label::LabelCommand::Usage) => Scope::Group("label"),
        Top::Team(team::TeamCommand::Usage) => Scope::Group("team"),
        Top::User(user::UserCommand::Usage) => Scope::Group("user"),
        Top::Cycle(args) if matches!(args.command, Some(cycle::CycleCommand::Usage)) => {
            Scope::Group("cycle")
        }
        Top::Document(document::DocumentCommand::Usage) => Scope::Group("document"),
        Top::Cache(cache::CacheCommand::Usage) => Scope::Group("cache"),
        Top::Webhook(webhook::WebhookCommand::Usage) => Scope::Group("webhook"),
        _ => return None,
    })
}

#[derive(Serialize)]
struct UsageOut<'a> {
    usage: &'a str,
}

pub fn run(out: Output, scope: Scope) -> Result<()> {
    let root = Cli::command();
    let text = match scope {
        Scope::All => overview(&root),
        Scope::Group(name) => group(&root, name)
            .ok_or_else(|| CliError::general(format!("no command group named {name:?}")))?,
    };
    out.emit(&UsageOut { usage: &text }, || text.clone(), || text.clone());
    Ok(())
}

// ------------------------------------------------------------------ the hand-written layer

/// How a command touches things. A command not named below is a write: the safe default, so
/// a command added later is never taken for a read until it is listed here.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// Changes nothing.
    Read,
    /// Changes only this machine (the configuration, the cache, a downloaded file).
    Local,
    /// Changes data in Linear.
    Write,
}

/// The subcommands that only read.
const READS: &[&str] = &[
    "list",
    "view",
    "search",
    "status-updates",
    "skeleton",
    "show",
    "whoami",
    "verify",
];

/// The subcommands that change only this machine.
const LOCALS: &[&str] = &["add", "login", "migrate", "refresh", "clear", "download"];

fn kind(name: &str) -> Kind {
    if READS.contains(&name) {
        Kind::Read
    } else if LOCALS.contains(&name) {
        Kind::Local
    } else {
        Kind::Write
    }
}

/// What an exit code means. Exhaustive, so a new code cannot be left without a line.
fn exit_meaning(code: ErrorCode) -> &'static str {
    match code {
        ErrorCode::General => "error",
        ErrorCode::Usage => "usage (bad flag, unknown or ambiguous name)",
        ErrorCode::Auth => "credentials",
        ErrorCode::WriteDenied => "ownership rules refused the write",
        ErrorCode::Validation => "a validator refused the write",
        ErrorCode::AuditFindings => "`audit --fail-on` found something",
    }
}

/// A line or two about a group that its flags do not say.
fn notes(group: &str) -> &'static [&'static str] {
    match group {
        "workspace" => &[
            "A workspace is a name in the configuration; `-w NAME` (or LINEAR_WORKSPACE, or a `.linear.toml`) picks it for any command. Credentials are never printed.",
        ],
        "issue" => &[
            "Name an issue by identifier (KK-12) or id. Changing one needs it to be assigned to you or in a project you lead (exit 4 otherwise).",
            "`create` with a --source URL that is already attached returns that issue instead of a new one, so it is safe to run again.",
        ],
        "comment" => &[
            "Takes the id of a comment (the `id` of `issue view --json` under `comments`). `issue comment` writes a new one.",
        ],
        "project" => &[
            "Name a project by slug id, URL or name. Writes work only on a project you lead (exit 4, whatever the workspace's ownership setting).",
        ],
        "milestone" => &[
            "A milestone belongs to a project: pass --project. Writes follow the ownership of that project.",
        ],
        "initiative" => &["Writes follow the ownership rules (exit 4)."],
        "cycle" => &[
            "`linear cycle <DATE>` (a meeting's day, YYYY-MM-DD) prints the cycle that holds the day after it; exit 2, listing the cycles, when none does.",
        ],
        "cache" => &[
            "Hooks and the statusline read this cache. `--cached` on a read command uses it instead of asking Linear.",
        ],
        "webhook" => &[
            "`create` prints the signing secret once; keep it. `verify` checks a delivery's signature offline.",
        ],
        _ => &[],
    }
}

// ------------------------------------------------------------------ rendering

/// The first sentence of `text` on one line, kept to `max` characters (a full stop counts only
/// when a capital letter follows, so "i.e. like" is not the end).
fn sentence(text: &str, max: usize) -> String {
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let bytes = text.as_bytes();
    let mut end = text.len();
    for (i, _) in text.match_indices(". ") {
        if bytes.get(i + 2).is_some_and(u8::is_ascii_uppercase) {
            end = i;
            break;
        }
    }
    let line = text[..end].trim_end_matches('.');
    if line.chars().count() > max {
        let head: String = line.chars().take(max.saturating_sub(3)).collect();
        format!("{}...", head.trim_end())
    } else {
        line.to_owned()
    }
}

/// `text` cut to `max` characters at the last comma that fits (else at a word, with `...`).
fn fit(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_owned();
    }
    let head: String = text.chars().take(max).collect();
    match head.rfind(", ") {
        Some(i) => head[..i].to_owned(),
        None => sentence(text, max),
    }
}

/// What a command does, without its asides: up to the first ` (` or `: `.
fn short(text: &str, max: usize) -> String {
    let line = sentence(text, usize::MAX);
    let end = [" (", ": "]
        .iter()
        .filter_map(|stop| line.find(stop))
        .min()
        .unwrap_or(line.len());
    sentence(&line[..end], max)
}

fn about(cmd: &clap::Command) -> String {
    cmd.get_about().map(|a| a.to_string()).unwrap_or_default()
}

/// The subcommands to describe (clap adds a `help` one itself). A group's own `usage` is
/// left out: the group's text is what it prints.
fn subcommands(cmd: &clap::Command) -> impl Iterator<Item = &clap::Command> {
    cmd.get_subcommands()
        .filter(|c| c.get_name() != "help" && !c.is_hide_set())
        .filter(|c| c.get_name() != "usage" || cmd.get_name() == "linear")
}

fn shown_args(cmd: &clap::Command) -> impl Iterator<Item = &Arg> {
    cmd.get_arguments().filter(|a| {
        !a.is_hide_set() && !a.is_global_set() && !matches!(a.get_id().as_str(), "help" | "version")
    })
}

fn takes_value(arg: &Arg) -> bool {
    !matches!(
        arg.get_action(),
        ArgAction::SetTrue | ArgAction::SetFalse | ArgAction::Count
    )
}

fn value_name(arg: &Arg) -> String {
    arg.get_value_names()
        .and_then(|names| names.first())
        .map(|n| n.to_string())
        .unwrap_or_else(|| arg.get_id().as_str().to_uppercase().replace('-', "_"))
}

/// `--flag <VALUE>`, `-q, --quiet`, `<ARG>` or `[ARG]...`: how an argument is written.
fn spelling(arg: &Arg) -> String {
    if arg.is_positional() {
        let name = value_name(arg);
        let many = matches!(arg.get_action(), ArgAction::Append);
        let dots = if many { "..." } else { "" };
        return if arg.is_required_set() {
            format!("<{name}>{dots}")
        } else {
            format!("[{name}]{dots}")
        };
    }
    let mut s = String::new();
    if let Some(short) = arg.get_short() {
        s.push_str(&format!("-{short}, "));
    }
    s.push_str(&format!(
        "--{}",
        arg.get_long().unwrap_or(arg.get_id().as_str())
    ));
    if takes_value(arg) {
        let possible: Vec<String> = arg
            .get_possible_values()
            .iter()
            .filter(|v| !v.is_hide_set())
            .map(|v| v.get_name().to_owned())
            .collect();
        if (1..=8).contains(&possible.len()) {
            s.push_str(&format!(" <{}>", possible.join("|")));
        } else {
            s.push_str(&format!(" <{}>", value_name(arg)));
        }
    }
    s
}

fn help(arg: &Arg, max: usize) -> String {
    arg.get_help()
        .map(|h| sentence(&h.to_string(), max))
        .unwrap_or_default()
}

/// Whether the command tree has a flag called `name` on any command (or globally).
fn tree_has_flag(cmd: &clap::Command, name: &str) -> bool {
    cmd.get_arguments().any(|a| a.get_long() == Some(name))
        || cmd.get_subcommands().any(|c| tree_has_flag(c, name))
}

/// Every command path with its description, leaf commands only, as `issue list`.
fn kind_line(group: &clap::Command) -> String {
    let mut parts = Vec::new();
    for (label, want) in [
        ("read", Kind::Read),
        ("local", Kind::Local),
        ("write", Kind::Write),
    ] {
        let names: Vec<&str> = subcommands(group)
            .map(|c| c.get_name())
            .filter(|n| kind(n) == want)
            .collect();
        if !names.is_empty() {
            parts.push(format!("{label}: {}", names.join(" ")));
        }
    }
    parts.join(" | ")
}

/// The whole picture: every group, the global flags, the exit codes and the safety model.
pub fn overview(root: &clap::Command) -> String {
    let mut s = String::from(
        "linear: a command-line client for Linear. `linear <group> usage` lists a group's commands \
         and flags; `--help` has the full text.\n\nCOMMANDS\n",
    );
    for cmd in subcommands(root) {
        let name = cmd.get_name();
        s.push_str(&format!("  {name:<11} {}\n", short(&about(cmd), 56)));
        if subcommands(cmd).next().is_some() {
            s.push_str(&format!("              {}\n", kind_line(cmd)));
        }
    }
    s.push_str("  (read changes nothing, local changes only this machine, write changes Linear)\n");

    s.push_str("\nGLOBAL FLAGS\n");
    for arg in root
        .get_arguments()
        .filter(|a| a.is_global_set() && !a.is_hide_set())
    {
        let text = arg
            .get_help()
            .map(|h| fit(&short(&h.to_string(), usize::MAX), 56))
            .unwrap_or_default();
        s.push_str(&format!("  {:<22} {text}\n", spelling(arg)));
    }

    s.push_str("\nEXIT CODES\n  0 ok");
    for code in ErrorCode::ALL {
        s.push_str(&format!("; {} {}", code.exit_code(), exit_meaning(code)));
    }
    s.push('\n');

    s.push_str("\nSAFETY\n");
    s.push_str(
        "  A write is checked before anything is sent: ownership (you may change only your own \
         issues and projects you lead; exit 4), then the workspace's validators (exit 5).\n",
    );
    s.push_str(
        "  Names (states, projects, labels, users) are resolved first; an unknown or ambiguous \
         one is exit 2 with the candidates.\n",
    );
    s.push_str(
        "  A command that needs --yes cannot be undone; without it, nothing is sent (exit 2).\n",
    );
    if tree_has_flag(root, "dry-run") {
        s.push_str(
            "  Add --dry-run to a write to see its mutations without sending them; refusals and exit codes are the real ones.\n",
        );
    }
    s.push_str("  With --json, errors go to stderr as {\"error\":{\"code\",\"message\"}}.\n");
    s
}

/// One command: its path, kind and what it does, then its arguments.
fn command_block(path: &str, cmd: &clap::Command, out: &mut String) {
    let name = cmd.get_name();
    let kind = match kind(name) {
        Kind::Read => "read",
        Kind::Local => "local",
        Kind::Write => "write",
    };
    let mut flags = Vec::new();
    if shown_args(cmd).any(|a| a.get_long() == Some("yes")) {
        flags.push("needs --yes: cannot be undone");
    }
    if shown_args(cmd).any(|a| a.get_long() == Some("dry-run")) {
        flags.push("has --dry-run");
    }
    let flags = if flags.is_empty() {
        String::new()
    } else {
        format!(", {}", flags.join(", "))
    };
    out.push_str(&format!(
        "\n{path} [{kind}{flags}]\n  {}\n",
        short(&about(cmd), 110)
    ));
    for arg in shown_args(cmd) {
        let help = help(arg, 96);
        if help.is_empty() {
            out.push_str(&format!("    {}\n", spelling(arg)));
        } else {
            out.push_str(&format!("    {:<28}  {help}\n", spelling(arg)));
        }
    }
}

fn walk(path: &str, cmd: &clap::Command, out: &mut String) {
    for sub in subcommands(cmd) {
        let sub_path = format!("{path} {}", sub.get_name());
        if subcommands(sub).next().is_some() {
            walk(&sub_path, sub, out);
        } else {
            command_block(&sub_path, sub, out);
        }
    }
}

/// One group's commands and their flags. `None` when there is no such group.
pub fn group(root: &clap::Command, name: &str) -> Option<String> {
    let cmd = subcommands(root).find(|c| c.get_name() == name)?;
    let mut s = format!(
        "linear {name}: {}\nGlobal flags (-w, --json, --quiet, --fields, ...) apply to every command; see `linear usage`.\n",
        short(&about(cmd), 110)
    );
    for note in notes(name) {
        s.push_str(&format!("{note}\n"));
    }
    // `linear cycle <DATE>`: a positional of the group itself.
    for arg in shown_args(cmd).filter(|a| a.is_positional()) {
        s.push_str(&format!("\nlinear {name} {} [read]\n", spelling(arg)));
        s.push_str(&format!("  {}\n", help(arg, 110)));
    }
    walk(&format!("linear {name}"), cmd, &mut s);
    Some(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every command path of the tree, e.g. `["issue", "list"]`, `help` left out.
    fn paths(cmd: &clap::Command, prefix: &[String], out: &mut Vec<Vec<String>>) {
        for sub in subcommands(cmd) {
            let mut path = prefix.to_vec();
            path.push(sub.get_name().to_owned());
            out.push(path.clone());
            paths(sub, &path, out);
        }
    }

    fn tree() -> clap::Command {
        let mut root = Cli::command();
        root.build();
        root
    }

    fn all_paths() -> Vec<Vec<String>> {
        let mut all = Vec::new();
        paths(&tree(), &[], &mut all);
        all
    }

    #[test]
    fn the_overview_names_every_top_level_command_and_every_group_subcommand() {
        let root = tree();
        let text = overview(&root);
        let lines: Vec<&str> = text.lines().collect();
        let mut groups = 0;
        for top in subcommands(&root) {
            let name = top.get_name();
            let at = lines
                .iter()
                .position(|l| l.split_whitespace().next() == Some(name) && l.starts_with("  "))
                .unwrap_or_else(|| panic!("`linear usage` has no line for `{name}`"));
            if subcommands(top).next().is_none() {
                continue;
            }
            // The next line lists the group's commands by kind.
            let listed: Vec<&str> = lines[at + 1].split_whitespace().collect();
            for sub in subcommands(top) {
                assert!(
                    listed.contains(&sub.get_name()),
                    "`linear usage` does not list `{name} {}`: {}",
                    sub.get_name(),
                    lines[at + 1]
                );
            }
            groups += 1;
        }
        assert!(groups >= 15, "{groups} groups");
    }

    #[test]
    fn every_group_has_usage_text_with_every_command_below_it_and_each_flag() {
        let root = tree();
        let mut checked = 0;
        for top in subcommands(&root) {
            // A group is a command with subcommands; `usage` itself is the command that
            // prints this, and is described by the overview.
            if subcommands(top).next().is_none() {
                continue;
            }
            let name = top.get_name();
            let text = group(&root, name).unwrap_or_else(|| panic!("no usage for `{name}`"));
            let mut inner = Vec::new();
            paths(top, &[name.to_owned()], &mut inner);
            for path in inner {
                let leaf = tree();
                let cmd = path
                    .iter()
                    .skip(1)
                    .fold(leaf.find_subcommand(name).unwrap(), |c, n| {
                        c.find_subcommand(n).unwrap()
                    });
                if subcommands(cmd).next().is_some() {
                    continue;
                }
                let heading = format!("linear {}", path.join(" "));
                assert!(
                    text.contains(&format!("\n{heading} [")),
                    "`linear {name} usage` has no section for `{}`",
                    path.join(" ")
                );
                for arg in shown_args(cmd) {
                    let flag = match arg.get_long() {
                        Some(long) => format!("--{long}"),
                        None => spelling(arg),
                    };
                    assert!(
                        text.contains(&flag),
                        "`linear {name} usage` does not list {flag} of `{}`",
                        path.join(" ")
                    );
                }
                checked += 1;
            }
        }
        assert!(checked > 40, "the walk found only {checked} commands");
    }

    #[test]
    fn every_group_answers_to_usage_and_the_dispatch_knows_it() {
        // `requested` must map each group's `usage` subcommand to that group: parse it.
        use clap::Parser;
        let root = tree();
        let mut seen = 0;
        for top in subcommands(&root).filter(|t| subcommands(t).next().is_some()) {
            let name = top.get_name();
            let cli = Cli::try_parse_from(["linear", name, "usage"])
                .unwrap_or_else(|e| panic!("`linear {name} usage` does not parse: {e}"));
            match requested(&cli.command) {
                Some(Scope::Group(g)) => assert_eq!(g, name),
                _ => panic!("`linear {name} usage` is not handled"),
            }
            seen += 1;
        }
        assert!(seen >= 15, "{seen} groups");
        let cli = Cli::try_parse_from(["linear", "usage"]).unwrap();
        assert!(matches!(requested(&cli.command), Some(Scope::All)));
    }

    /// The hand-written lists name commands that exist.
    #[test]
    fn the_read_and_local_lists_have_no_stale_names() {
        let names: Vec<String> = all_paths()
            .into_iter()
            .map(|p| p.last().unwrap().clone())
            .collect();
        for name in READS.iter().chain(LOCALS) {
            assert!(
                names.iter().any(|n| n == name),
                "`{name}` is listed as read or local but no command has it"
            );
        }
        let groups: Vec<String> = subcommands(&tree())
            .map(|c| c.get_name().to_owned())
            .collect();
        for group in [
            "workspace",
            "issue",
            "comment",
            "project",
            "milestone",
            "initiative",
            "cycle",
            "cache",
            "webhook",
        ] {
            assert!(groups.iter().any(|g| g == group), "notes for `{group}`");
        }
    }

    #[test]
    fn the_exit_codes_are_the_ones_the_binary_uses() {
        let text = overview(&tree());
        for code in ErrorCode::ALL {
            assert!(
                text.contains(&format!("; {} {}", code.exit_code(), exit_meaning(code))),
                "{code}"
            );
        }
    }

    #[test]
    fn the_overview_fits_in_a_thousand_tokens() {
        // About four characters to a token; the limit leaves a margin.
        let text = overview(&tree());
        assert!(
            text.chars().count() < 3600,
            "`linear usage` is {} characters",
            text.chars().count()
        );
    }

    #[test]
    fn the_dry_run_sentence_appears_only_where_the_tree_has_the_flag() {
        let root = tree();
        let text = overview(&root);
        assert_eq!(
            text.contains("--dry-run"),
            tree_has_flag(&root, "dry-run"),
            "{text}"
        );
    }
}
