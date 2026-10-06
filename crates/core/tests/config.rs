//! Configuration parsing and workspace resolution.

use linear_core::config::*;
use linear_core::{Error, ErrorCode};

const TOML: &str = r#"
default = "main"

[workspaces.main]
url_key = "example"
default_team = "EX"

[workspaces.work]
url_key = "work-co"
auth = "oauth"
rules = ["template-sections", "source-attachment"]
allow_raw_mutation = true

[workspaces.theirs]
url_key = "someone-else"
"#;

fn config() -> Config {
    Config::parse(TOML).unwrap()
}

#[test]
fn parses_workspaces_with_defaults() {
    let c = config();
    assert_eq!(c.default.as_deref(), Some("main"));
    assert_eq!(c.names(), vec!["main", "theirs", "work"]);

    let main = c.get("main").unwrap();
    assert_eq!(main.url_key, "example");
    assert_eq!(main.default_team.as_deref(), Some("EX"));
    assert_eq!(main.auth, AuthMethod::ApiKey);
    assert!(main.rules.is_empty());
    assert!(!main.allow_raw_mutation);

    let work = c.get("work").unwrap();
    assert_eq!(work.auth, AuthMethod::Oauth);
    assert_eq!(
        work.rules,
        vec![Rule::TemplateSections, Rule::SourceAttachment]
    );
    assert!(work.allow_raw_mutation);
}

#[test]
fn an_empty_file_is_an_empty_config() {
    let c = Config::parse("").unwrap();
    assert!(c.default.is_none());
    assert!(c.workspaces.is_empty());
}

#[test]
fn unknown_keys_and_rules_are_errors_not_ignored() {
    for bad in [
        "[workspaces.a]\nurl_key = \"a\"\ntypo = 1\n",
        "[workspaces.a]\nurl_key = \"a\"\nrules = [\"no-such-rule\"]\n",
        "[workspaces.a]\nurl_key = \"a\"\nauth = \"password\"\n",
        "[workspaces.a]\n", // url_key is required
        "unknown_top_level = 1\n",
    ] {
        let err = Config::parse(bad).unwrap_err();
        assert!(matches!(err, Error::Config(_)), "{bad:?} -> {err:?}");
    }
}

#[test]
fn the_default_must_name_a_defined_workspace() {
    let err = Config::parse("default = \"ghost\"\n[workspaces.a]\nurl_key = \"a\"\n").unwrap_err();
    assert!(err.to_string().contains("ghost"));
}

#[test]
fn workspace_names_are_restricted_because_they_become_file_names() {
    for ok in ["a", "ken109", "work-2", "my_ws"] {
        validate_workspace_name(ok).unwrap();
    }
    for bad in [
        "",
        "A",
        "../x",
        "a/b",
        "a.b",
        "-a",
        "_a",
        "a b",
        &"x".repeat(65),
    ] {
        assert!(validate_workspace_name(bad).is_err(), "{bad:?}");
    }
    let err = Config::parse("[workspaces.\"../evil\"]\nurl_key = \"a\"\n").unwrap_err();
    assert!(matches!(err, Error::Config(_)));
}

#[test]
fn round_trips_through_toml() {
    let c = config();
    let text = toml_edit::ser::to_string(&c).unwrap();
    assert_eq!(Config::parse(&text).unwrap(), c);
}

// ------------------------------------------------------------------ audit settings

const AUDIT_TOML: &str = r#"
[workspaces.a]
url_key = "a"
rules = ["template-sections", "label-groups-exclusive"]

[workspaces.a.audit]
stale_days = 3
status_update_days = 30

[workspaces.b]
url_key = "b"

[workspaces.c]
url_key = "c"

[workspaces.c.audit]
stale_days = 10
"#;

#[test]
fn audit_settings_become_an_audit_config_over_the_defaults() {
    use linear_core::audit::AuditConfig;
    let c = Config::parse(AUDIT_TOML).unwrap();

    let a = AuditConfig::from_workspace(c.get("a").unwrap());
    assert_eq!(a.stale_days, 3);
    assert_eq!(a.status_update_days, 30);
    assert_eq!(
        a.validators,
        [Rule::TemplateSections, Rule::LabelGroupsExclusive]
    );

    assert_eq!(
        AuditConfig::from_workspace(c.get("b").unwrap()),
        AuditConfig::default()
    );

    let c = AuditConfig::from_workspace(c.get("c").unwrap());
    assert_eq!((c.stale_days, c.status_update_days), (10, 14));
}

#[test]
fn audit_settings_reject_unknown_keys_and_zero_days() {
    for bad in [
        "[workspaces.a]\nurl_key = \"a\"\n[workspaces.a.audit]\nstale = 3\n",
        "[workspaces.a]\nurl_key = \"a\"\n[workspaces.a.audit]\nstale_days = 0\n",
        "[workspaces.a]\nurl_key = \"a\"\n[workspaces.a.audit]\nstatus_update_days = 0\n",
        "[workspaces.a]\nurl_key = \"a\"\n[workspaces.a.audit]\nstale_days = -1\n",
    ] {
        let err = Config::parse(bad).unwrap_err();
        assert!(matches!(err, Error::Config(_)), "{bad:?} -> {err:?}");
    }
}

#[test]
fn audit_settings_round_trip_and_stay_out_of_the_file_when_unset() {
    let c = Config::parse(AUDIT_TOML).unwrap();
    let text = toml_edit::ser::to_string(&c).unwrap();
    assert_eq!(Config::parse(&text).unwrap(), c);
    assert!(!text.contains("[workspaces.b.audit]"), "{text}");
}

// ------------------------------------------------------------------ resolution

fn pick(sel: Selectors<'_>) -> (String, Source) {
    let c = config();
    let r = resolve(sel, &c).unwrap();
    (r.name, r.source)
}

#[test]
fn resolution_order_is_flag_then_env_then_repo_file_then_default() {
    let repo = "workspace = \"theirs\"\n";
    let all = Selectors {
        flag: Some("work"),
        env: Some("theirs"),
        repo_file: Some(repo),
    };
    assert_eq!(pick(all), ("work".into(), Source::Flag));
    assert_eq!(
        pick(Selectors { flag: None, ..all }),
        ("theirs".into(), Source::Env)
    );
    assert_eq!(
        pick(Selectors {
            flag: None,
            env: None,
            ..all
        }),
        ("theirs".into(), Source::RepoFile)
    );
    assert_eq!(
        pick(Selectors::default()),
        ("main".into(), Source::ConfigDefault)
    );
}

#[test]
fn empty_selectors_count_as_unset() {
    let got = pick(Selectors {
        flag: Some(""),
        env: Some("  "),
        repo_file: None,
    });
    assert_eq!(got, ("main".into(), Source::ConfigDefault));
}

#[test]
fn an_unknown_workspace_names_where_it_came_from_and_what_exists() {
    let c = config();
    let err = resolve(
        Selectors {
            env: Some("ghost"),
            ..Default::default()
        },
        &c,
    )
    .unwrap_err();
    assert_eq!(err.code(), ErrorCode::Usage);
    let msg = err.to_string();
    assert!(
        msg.contains("ghost") && msg.contains("LINEAR_WORKSPACE") && msg.contains("main"),
        "{msg}"
    );
}

#[test]
fn nothing_selected_and_no_default_is_a_usage_error() {
    let c = Config::parse("[workspaces.a]\nurl_key = \"a\"\n").unwrap();
    let err = resolve(Selectors::default(), &c).unwrap_err();
    assert_eq!(err.code(), ErrorCode::Usage);
    assert!(err.to_string().contains("--workspace"));

    let err = resolve(Selectors::default(), &Config::default()).unwrap_err();
    assert!(err.to_string().contains("workspace add"));
}

#[test]
fn a_malformed_repo_file_is_a_config_error() {
    let c = config();
    let err = resolve(
        Selectors {
            repo_file: Some("workspace = 3\n"),
            ..Default::default()
        },
        &c,
    )
    .unwrap_err();
    assert!(matches!(err, Error::Config(_)));
    let err = resolve(
        Selectors {
            repo_file: Some("other = \"x\"\n"),
            ..Default::default()
        },
        &c,
    )
    .unwrap_err();
    assert!(matches!(err, Error::Config(_)));
}

// ------------------------------------------------------------- source_kinds

#[test]
fn source_kinds_belong_to_the_source_attachment_rule() {
    let ok = Config::parse(
        "[workspaces.w]\nurl_key = \"w\"\nrules = [\"source-attachment\"]\nsource_kinds = [\"slack\", \"repo\"]\n",
    )
    .unwrap();
    assert_eq!(ok.get("w").unwrap().source_kinds, ["slack", "repo"]);

    // Absent by default, and not written back when empty.
    let none = Config::parse("[workspaces.w]\nurl_key = \"w\"\n").unwrap();
    assert!(none.get("w").unwrap().source_kinds.is_empty());
    assert!(!toml_edit::ser::to_string(&none)
        .unwrap()
        .contains("source_kinds"));

    // It feeds the audit's settings.
    let audit = linear_core::audit::AuditConfig::from_workspace(ok.get("w").unwrap());
    assert_eq!(audit.source_kinds, ["slack", "repo"]);
}

#[test]
fn source_kinds_without_the_rule_or_with_an_empty_kind_is_an_error() {
    for bad in [
        "[workspaces.w]\nurl_key = \"w\"\nsource_kinds = [\"slack\"]\n",
        "[workspaces.w]\nurl_key = \"w\"\nrules = [\"label-groups-exclusive\"]\nsource_kinds = [\"slack\"]\n",
        "[workspaces.w]\nurl_key = \"w\"\nrules = [\"source-attachment\"]\nsource_kinds = [\"slack\", \"  \"]\n",
        "[workspaces.w]\nurl_key = \"w\"\nrules = [\"source-attachment\"]\nsource_kind = [\"slack\"]\n",
        "[workspaces.w]\nurl_key = \"w\"\nrules = [\"source-attachment\"]\nsource_kinds = [1]\n",
    ] {
        let err = Config::parse(bad).unwrap_err();
        assert!(matches!(err, Error::Config(_)), "{bad:?} -> {err:?}");
    }
}
