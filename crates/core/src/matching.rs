//! Resolving what a person typed (a name, a key, an id, a URL) to one entity.
//!
//! Resolution is strict: a name that matches nothing, or more than one thing,
//! is an error that lists the candidates. Nothing is ever picked "close enough".

use crate::error::{Error, Result};
use crate::types::*;

const MAX_LISTED: usize = 20;

/// Pick the one row `reference` names.
///
/// Rows that match exactly win; failing that, rows that match ignoring case.
/// More than one match at the winning level is ambiguous.
///
/// * `exact` / `folded` say whether a row matches the reference exactly /
///   ignoring case.
/// * `describe` names a row in the "available" list of an error.
pub fn pick<'a, T>(
    rows: &'a [T],
    what: &str,
    reference: &str,
    exact: impl Fn(&T) -> bool,
    folded: impl Fn(&T) -> bool,
    describe: impl Fn(&T) -> String,
) -> Result<&'a T> {
    for matches in [
        rows.iter().filter(|r| exact(r)).collect::<Vec<_>>(),
        rows.iter().filter(|r| folded(r)).collect::<Vec<_>>(),
    ] {
        match matches.as_slice() {
            [] => continue,
            [one] => return Ok(one),
            many => {
                return Err(Error::Usage(format!(
                    "{what} {reference:?} is ambiguous; it matches: {}",
                    join(many.iter().map(|r| describe(r)))
                )))
            }
        }
    }
    let available = if rows.is_empty() {
        "none".to_owned()
    } else {
        join(rows.iter().map(&describe))
    };
    Err(Error::Usage(format!(
        "no {what} {reference:?} (available: {available})"
    )))
}

fn join(items: impl Iterator<Item = String>) -> String {
    let all: Vec<String> = items.collect();
    if all.len() > MAX_LISTED {
        format!(
            "{} and {} more",
            all[..MAX_LISTED].join(", "),
            all.len() - MAX_LISTED
        )
    } else {
        all.join(", ")
    }
}

/// The last path segment of a Linear URL after `marker`, or the input itself.
///
/// `https://linear.app/acme/project/algo-trade-da0598c291be/overview` with
/// marker `project` gives `algo-trade-da0598c291be`.
pub fn slug_from_url(reference: &str, marker: &str) -> String {
    let reference = reference.trim();
    if !(reference.starts_with("http://") || reference.starts_with("https://")) {
        return reference.to_owned();
    }
    let path = reference
        .split(['?', '#'])
        .next()
        .unwrap_or(reference)
        .trim_end_matches('/');
    let mut parts = path.split('/');
    while let Some(p) = parts.next() {
        if p == marker {
            if let Some(slug) = parts.next() {
                return slug.to_owned();
            }
        }
    }
    reference.to_owned()
}

fn same_ignoring_case(a: &str, b: &str) -> bool {
    a.to_lowercase() == b.to_lowercase()
}

/// A project by id, slug id, URL or name.
pub fn match_project<'a>(rows: &'a [ProjectRef], reference: &str) -> Result<&'a ProjectRef> {
    let slug = slug_from_url(reference, "project");
    // A URL's slug is `<name>-<slugId>`, and the same shape is accepted bare.
    let by_id = |p: &ProjectRef| {
        p.id.inner() == slug
            || p.slug_id == slug
            || slug
                .strip_suffix(p.slug_id.as_str())
                .is_some_and(|rest| rest.ends_with('-'))
    };
    if let Some(hit) = rows.iter().find(|p| by_id(p)) {
        return Ok(hit);
    }
    pick(
        rows,
        "project",
        reference,
        |p| p.name == reference,
        |p| same_ignoring_case(&p.name, reference),
        |p| p.name.clone(),
    )
}

/// An initiative by id, slug id, URL or name.
pub fn match_initiative<'a>(rows: &'a [Initiative], reference: &str) -> Result<&'a Initiative> {
    let slug = slug_from_url(reference, "initiative");
    let by_id = |i: &Initiative| {
        i.id.inner() == slug
            || i.slug_id == slug
            || slug
                .strip_suffix(i.slug_id.as_str())
                .is_some_and(|rest| rest.ends_with('-'))
    };
    if let Some(hit) = rows.iter().find(|i| by_id(i)) {
        return Ok(hit);
    }
    pick(
        rows,
        "initiative",
        reference,
        |i| i.name == reference,
        |i| same_ignoring_case(&i.name, reference),
        |i| i.name.clone(),
    )
}

/// A webhook by id, label or URL. Several webhooks can deliver to one URL, so a
/// URL that names more than one is ambiguous like any other repeated name.
pub fn match_webhook<'a>(rows: &'a [Webhook], reference: &str) -> Result<&'a Webhook> {
    let url_is = |w: &Webhook, same: fn(&str, &str) -> bool| {
        w.url.as_deref().is_some_and(|u| same(u, reference))
    };
    let label_is = |w: &Webhook, same: fn(&str, &str) -> bool| {
        w.label.as_deref().is_some_and(|l| same(l, reference))
    };
    pick(
        rows,
        "webhook",
        reference,
        |w| w.id.inner() == reference || label_is(w, |a, b| a == b) || url_is(w, |a, b| a == b),
        |w| label_is(w, same_ignoring_case) || url_is(w, same_ignoring_case),
        |w| {
            let name = w
                .label
                .as_deref()
                .or(w.url.as_deref())
                .unwrap_or("(no label)");
            format!("{name} ({})", w.id.inner())
        },
    )
}

/// A milestone of one project, by id or name.
pub fn match_milestone<'a>(rows: &'a [Milestone], reference: &str) -> Result<&'a Milestone> {
    pick(
        rows,
        "milestone",
        reference,
        |m| m.id.inner() == reference || m.name == reference,
        |m| same_ignoring_case(&m.name, reference),
        |m| m.name.clone(),
    )
}

/// A team by key or name.
pub fn match_team<'a>(rows: &'a [Team], reference: &str) -> Result<&'a Team> {
    pick(
        rows,
        "team",
        reference,
        |t| t.key == reference || t.id.inner() == reference,
        |t| same_ignoring_case(&t.key, reference) || same_ignoring_case(&t.name, reference),
        |t| t.key.clone(),
    )
}

/// A workflow state of one team, by id or name.
pub fn match_state<'a>(rows: &'a [WorkflowState], reference: &str) -> Result<&'a WorkflowState> {
    pick(
        rows,
        "state",
        reference,
        |s| s.id.inner() == reference || s.name == reference,
        |s| same_ignoring_case(&s.name, reference),
        |s| s.name.clone(),
    )
}

/// A user by email, name or display name. `me` is resolved by the caller.
pub fn match_user<'a>(rows: &'a [User], reference: &str) -> Result<&'a User> {
    pick(
        rows,
        "user",
        reference,
        |u| u.email == reference || u.id.inner() == reference,
        |u| {
            same_ignoring_case(&u.email, reference)
                || same_ignoring_case(&u.name, reference)
                || same_ignoring_case(&u.display_name, reference)
        },
        |u| u.name.clone(),
    )
}

/// Labels named `reference` (a name or an id). The same name can exist in
/// several teams, so this returns every match rather than picking one.
pub fn match_labels<'a>(rows: &'a [Label], reference: &str) -> Result<Vec<&'a Label>> {
    let exact: Vec<&Label> = rows
        .iter()
        .filter(|l| l.id.inner() == reference || l.name == reference)
        .collect();
    if !exact.is_empty() {
        return Ok(exact);
    }
    let folded: Vec<&Label> = rows
        .iter()
        .filter(|l| same_ignoring_case(&l.name, reference))
        .collect();
    if !folded.is_empty() {
        return Ok(folded);
    }
    Err(Error::Usage(format!(
        "no label {reference:?} (available: {})",
        if rows.is_empty() {
            "none".to_owned()
        } else {
            join(rows.iter().map(label_path))
        }
    )))
}

/// `group/name` for a label inside a group, otherwise just the name.
pub fn label_path(label: &Label) -> String {
    match &label.parent {
        Some(group) => format!("{}/{}", group.name, label.name),
        None => label.name.clone(),
    }
}

/// An issue template by name.
pub fn match_template<'a>(rows: &'a [Template], reference: &str) -> Result<&'a Template> {
    pick(
        rows,
        "template",
        reference,
        |t| t.name == reference || t.id.inner() == reference,
        |t| same_ignoring_case(&t.name, reference),
        |t| match &t.team {
            Some(team) => format!("{} ({})", t.name, team.key),
            None => t.name.clone(),
        },
    )
}
