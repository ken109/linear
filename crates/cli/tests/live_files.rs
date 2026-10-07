//! `file upload`, `file download` and `issue attach-file` against the real
//! sandbox workspace. Ignored by default; they need the sandbox credentials in
//! the environment:
//!
//! ```sh
//! set -a; . ~/.config/linear-dev/sandbox.env; set +a
//! cargo test -p linear --test live_files -- --ignored
//! ```
//!
//! The issue is created under a unique name and trashed when the test ends. The
//! CLI has no command that deletes an uploaded file (only the cleanup after a
//! failed attach does), so the test deletes what it uploaded with the very
//! mutation that cleanup sends (`file_upload_delete`), through `linear api`
//! with `allow_raw_mutation` turned on for the scratch configuration. That also
//! checks the mutation against the real schema.

mod common;

use common::*;
use linear_core::files::file_upload_delete;
use linear_core::wire::build_request;
use serde_json::Value;

/// A 1x1 transparent PNG.
const PNG: &[u8] = &[
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1f, 0x15, 0xc4,
    0x89, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9c, 0x63, 0xf8, 0xff, 0xff, 0x3f,
    0x00, 0x05, 0xfe, 0x02, 0xfe, 0xa7, 0x35, 0x81, 0x84, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4e,
    0x44, 0xae, 0x42, 0x60, 0x82,
];

struct Live {
    sb: Sandbox,
    key: String,
}

impl Live {
    fn new() -> Live {
        let key = std::env::var("LINEAR_API_KEY_SANDBOX").expect("LINEAR_API_KEY_SANDBOX");
        let sb = Sandbox::new();
        let o = sb.run(
            &[
                "workspace",
                "add",
                "sandbox",
                "--url-key",
                "ken109-sandbox",
                "--team",
                "SAND",
            ],
            None,
            &[],
        );
        assert_eq!(code(&o), 0, "{}", stderr(&o));
        // Only the cleanup of uploaded files needs a raw mutation.
        let path = sb.config_dir().join("workspaces.toml");
        let mut text = std::fs::read_to_string(&path).unwrap();
        text.push_str("\nallow_raw_mutation = true\n");
        std::fs::write(&path, text).unwrap();
        Live { sb, key }
    }

    fn run(&self, args: &[&str]) -> std::process::Output {
        let o = self
            .sb
            .run(args, None, &[("LINEAR_API_KEY_SANDBOX", &self.key)]);
        assert!(!stdout(&o).contains(&self.key) && !stderr(&o).contains(&self.key));
        o
    }

    fn json(&self, args: &[&str]) -> Value {
        let mut full = args.to_vec();
        full.push("--json");
        let o = self.run(&full);
        assert_eq!(code(&o), 0, "{args:?}: {}", stderr(&o));
        serde_json::from_str(&stdout(&o)).unwrap_or_else(|e| panic!("{args:?}: {e}"))
    }

    fn write(&self, name: &str, bytes: &[u8]) -> String {
        let path = self.sb.cwd().join(name);
        std::fs::write(&path, bytes).unwrap();
        path.to_string_lossy().into_owned()
    }
}

/// Trashes the issue and deletes the uploaded files, whether the test passed or not.
struct Cleanup<'a> {
    live: &'a Live,
    issues: Vec<String>,
    assets: Vec<String>,
}

impl Drop for Cleanup<'_> {
    fn drop(&mut self) {
        for issue in &self.issues {
            let _ = self.live.run(&["issue", "delete", issue]);
        }
        let query = build_request(&file_upload_delete("x")).query;
        for url in &self.assets {
            let o = self.live.run(&[
                "api",
                "--mutation",
                &query,
                "--var",
                &format!("assetUrl={url}"),
                "--json",
            ]);
            if code(&o) != 0 || !stdout(&o).contains("true") {
                eprintln!("could not delete {url}: {}{}", stdout(&o), stderr(&o));
            }
        }
    }
}

#[test]
#[ignore = "needs LINEAR_API_KEY_SANDBOX and network access"]
fn files_upload_attach_and_download_end_to_end() {
    let live = Live::new();
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let mut cleanup = Cleanup {
        live: &live,
        issues: Vec::new(),
        assets: Vec::new(),
    };

    // upload: the asset URL is Linear's file host, and the markdown embeds it as an image.
    let shot = live.write("shot.png", PNG);
    let up = live.json(&["file", "upload", &shot]);
    let asset = up["assetUrl"].as_str().unwrap().to_owned();
    cleanup.assets.push(asset.clone());
    assert!(asset.starts_with("https://uploads.linear.app/"), "{up}");
    assert_eq!(up["contentType"], "image/png");
    assert_eq!(up["size"], PNG.len());
    assert_eq!(up["markdown"], format!("![shot.png]({asset})"));

    // download: the credential goes to uploads.linear.app, and the bytes are the same.
    let saved = live.sb.cwd().join("downloaded.png");
    let down = live.json(&["file", "download", &asset, "--output", "downloaded.png"]);
    assert_eq!(down["size"], PNG.len());
    assert_eq!(std::fs::read(&saved).unwrap(), PNG);
    // Nothing is replaced without --force.
    let o = live.run(&["file", "download", &asset, "-o", "downloaded.png"]);
    assert_eq!(code(&o), 2, "{}", stderr(&o));

    // attach-file: attached to a fresh issue, listed on it, and downloadable from its URL.
    let made = live.json(&[
        "issue",
        "create",
        "--title",
        &format!("live attach-file {stamp}"),
        "--project",
        "Fixture Project",
    ]);
    let id = made["identifier"].as_str().unwrap().to_owned();
    cleanup.issues.push(id.clone());
    let doc = live.write("notes.txt", b"hello from a live test\n");
    let attached = live.json(&["issue", "attach-file", &id, &doc, "--title", "Live notes"]);
    let doc_asset = attached["url"].as_str().unwrap().to_owned();
    cleanup.assets.push(doc_asset.clone());
    assert_eq!(attached["issue"], id.as_str());
    assert_eq!(attached["title"], "Live notes");
    assert!(doc_asset.starts_with("https://uploads.linear.app/"));
    assert_eq!(attached["markdown"], format!("[notes.txt]({doc_asset})"));
    let viewed = live.json(&["issue", "view", &id]);
    let attachments = viewed["attachments"]["nodes"].as_array().unwrap();
    assert!(
        attachments
            .iter()
            .any(|a| a["url"] == doc_asset.as_str() && a["title"] == "Live notes"),
        "not attached: {viewed}"
    );
    let o = live.run(&["file", "download", &doc_asset, "-o", "-"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(o.stdout, b"hello from a live test\n");

    // An image embedded in a comment (the markdown `file upload` printed).
    let comment = live.write(
        "comment.md",
        format!("Screenshot:\n\n{}\n", up["markdown"].as_str().unwrap()).as_bytes(),
    );
    let commented = live.json(&["issue", "comment", &id, "--body-file", &comment]);
    assert!(commented["url"].as_str().is_some());

    // The cleanup mutation really removes a file: deleted, it can no longer be downloaded.
    let query = build_request(&file_upload_delete("x")).query;
    let deleted = live.json(&[
        "api",
        "--mutation",
        &query,
        "--var",
        &format!("assetUrl={asset}"),
    ]);
    assert_eq!(deleted["fileUploadDangerouslyDelete"]["success"], true);
    cleanup.assets.retain(|a| a != &asset);
    // Linear reports success at once, but a cached copy may stay downloadable for a while, so
    // this only waits (briefly) to see it go and does not insist.
    let mut gone = false;
    for _ in 0..5 {
        let o = live.run(&["file", "download", &asset, "-o", "gone.png"]);
        if code(&o) != 0 {
            gone = true;
            break;
        }
        std::fs::remove_file(live.sb.cwd().join("gone.png")).unwrap();
        std::thread::sleep(std::time::Duration::from_secs(2));
    }
    println!("deleted file no longer downloadable within 10 s: {gone}");

    // A file over the limit is refused before anything is sent.
    let big = live.sb.cwd().join("big.bin");
    std::fs::File::create(&big)
        .unwrap()
        .set_len(25 * 1024 * 1024 + 1)
        .unwrap();
    let o = live.run(&["issue", "attach-file", &id, &big.to_string_lossy()]);
    assert_eq!(code(&o), 2, "{}", stderr(&o));
    assert!(stderr(&o).contains("25 MiB"), "{}", stderr(&o));
}
