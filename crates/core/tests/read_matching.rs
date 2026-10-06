//! Resolving references (names, slugs, URLs) to one entity.

use linear_core::matching::{match_project, slug_from_url};
use linear_core::types::ProjectRef;

fn project(id: &str, slug: &str, name: &str) -> ProjectRef {
    ProjectRef {
        id: cynic::Id::new(id),
        slug_id: slug.to_owned(),
        name: name.to_owned(),
        url: format!("https://linear.app/example/project/{slug}"),
    }
}

fn rows() -> Vec<ProjectRef> {
    vec![
        project("id-1", "da0598c291be", "Algo Trade"),
        project("id-2", "1d9f0aee38e7", "Fixture Project"),
        project("id-3", "aaaaaaaaaaaa", "Twin"),
        project("id-4", "bbbbbbbbbbbb", "twin"),
    ]
}

#[test]
fn a_project_is_found_by_id_slug_url_or_name() {
    let rows = rows();
    for reference in [
        "id-1",
        "da0598c291be",
        "algo-trade-da0598c291be",
        "https://linear.app/example/project/algo-trade-da0598c291be/overview",
        "Algo Trade",
        "algo trade",
    ] {
        assert_eq!(
            match_project(&rows, reference).unwrap().id.inner(),
            "id-1",
            "{reference}"
        );
    }
}

#[test]
fn an_unknown_project_lists_what_exists() {
    let err = match_project(&rows(), "Nope").unwrap_err().to_string();
    assert!(err.contains("no project \"Nope\""), "{err}");
    assert!(err.contains("Algo Trade, Fixture Project"), "{err}");

    let err = match_project(&[], "Nope").unwrap_err().to_string();
    assert!(err.contains("available: none"), "{err}");
}

#[test]
fn an_exact_name_beats_a_case_insensitive_one_and_ties_are_ambiguous() {
    let rows = rows();
    assert_eq!(match_project(&rows, "Twin").unwrap().id.inner(), "id-3");
    assert_eq!(match_project(&rows, "twin").unwrap().id.inner(), "id-4");
    let err = match_project(&rows, "TWIN").unwrap_err().to_string();
    assert!(err.contains("ambiguous"), "{err}");
}

#[test]
fn slugs_are_taken_from_urls_only() {
    assert_eq!(
        slug_from_url(
            "https://linear.app/x/project/abc-123/overview?a=1",
            "project"
        ),
        "abc-123"
    );
    assert_eq!(slug_from_url("abc-123", "project"), "abc-123");
    assert_eq!(
        slug_from_url("https://linear.app/x/other/abc", "project"),
        "https://linear.app/x/other/abc"
    );
}
