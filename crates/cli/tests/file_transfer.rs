//! `file upload`, `file download` and `issue attach-file` against a mock Linear
//! (GraphQL, by operation name) and a mock file storage (the signed upload URL
//! and the download host), which records every request it gets.

mod common;
mod read_support;
mod write_support;

use common::*;
use read_support::*;
use serde_json::json;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use write_support::*;

const ASSET: &str = "https://uploads.linear.app/org/0001/0002";
const ATTACHMENT: &str = "00000000-0000-4000-8000-0000000000a1";

// ------------------------------------------------------------------ the file storage

#[derive(Debug, Clone)]
struct Hit {
    method: String,
    path: String,
    headers: HashMap<String, String>,
    body: Vec<u8>,
}

/// Answers `PUT /upload/*` and `GET /files/*` from a table and records everything it sees.
struct Storage {
    base: String,
    hits: Arc<Mutex<Vec<Hit>>>,
}

impl Storage {
    /// `files`: path -> (status, body). A path that is not listed answers 404.
    fn start(files: Vec<(&'static str, u16, Vec<u8>)>) -> Storage {
        let server = tiny_http::Server::http("127.0.0.1:0").expect("bind");
        let base = format!("http://{}", server.server_addr().to_ip().expect("ip"));
        let hits = Arc::new(Mutex::new(Vec::<Hit>::new()));
        let log = Arc::clone(&hits);
        std::thread::spawn(move || {
            for mut req in server.incoming_requests() {
                let mut body = Vec::new();
                let _ = std::io::Read::read_to_end(req.as_reader(), &mut body);
                let headers = req
                    .headers()
                    .iter()
                    .map(|h| {
                        (
                            h.field.as_str().as_str().to_ascii_lowercase(),
                            h.value.to_string(),
                        )
                    })
                    .collect();
                let path = req.url().to_owned();
                log.lock().unwrap().push(Hit {
                    method: req.method().to_string(),
                    path: path.clone(),
                    headers,
                    body,
                });
                let (status, body) = files
                    .iter()
                    .find(|(p, _, _)| *p == path)
                    .map(|(_, s, b)| (*s, b.clone()))
                    .unwrap_or((404, b"not found".to_vec()));
                let _ = req.respond(tiny_http::Response::from_data(body).with_status_code(status));
            }
        });
        Storage { base, hits }
    }

    fn hits(&self) -> Vec<Hit> {
        self.hits.lock().unwrap().clone()
    }

    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base)
    }
}

// ------------------------------------------------------------------ the GraphQL side

fn upload_reply(storage: &Storage, filename: &str, content_type: &str, size: usize) -> Reply {
    data(json!({ "fileUpload": { "success": true, "uploadFile": {
        "assetUrl": ASSET, "contentType": content_type, "filename": filename, "size": size,
        "uploadUrl": storage.url("/upload/signed?sig=abc"),
        "headers": [
            { "key": "Content-Type", "value": content_type },
            { "key": "x-goog-meta-test", "value": "yes" },
        ],
    }}}))
}

fn attached() -> Reply {
    data(
        json!({ "attachmentCreate": { "success": true, "attachment": {
            "id": ATTACHMENT, "title": "shot.png", "subtitle": "image/png, 4 B", "url": ASSET,
            "sourceType": null, "metadata": {}, "createdAt": "2026-10-07T01:00:00.000Z"
        }}}),
    )
}

fn mine() -> View {
    view("EX-23")
        .assigned_to(Some(ALICE))
        .in_project(PROJECT, Some(ALICE))
}

fn routes(
    storage: &Storage,
    extra: Vec<(&'static str, Vec<Reply>)>,
) -> Vec<(&'static str, Vec<Reply>)> {
    let mut routes: Vec<(&'static str, Vec<Reply>)> = vec![
        ("Whoami", vec![whoami()]),
        ("IssueWriteView", vec![mine().reply()]),
        (
            "FileUpload",
            vec![upload_reply(storage, "shot.png", "image/png", 4)],
        ),
        ("AttachmentCreate", vec![attached()]),
        ("AttachmentTargetsQuery", vec![attachment_targets(&[])]),
        (
            "FileUploadDangerouslyDelete",
            vec![data(
                json!({ "fileUploadDangerouslyDelete": { "success": true } }),
            )],
        ),
    ];
    for (op, replies) in extra {
        routes.retain(|(o, _)| *o != op);
        routes.push((op, replies));
    }
    routes
}

const PNG: &[u8] = b"\x89PNG";

fn write_bytes(sb: &Sandbox, name: &str, bytes: &[u8]) -> String {
    let path = sb.cwd().join(name);
    std::fs::write(&path, bytes).unwrap();
    path.to_string_lossy().into_owned()
}

// ------------------------------------------------------------------ file upload

#[test]
fn upload_puts_the_bytes_to_the_signed_url_with_the_headers_linear_named() {
    let sb = workspace();
    let storage = Storage::start(vec![("/upload/signed?sig=abc", 200, vec![])]);
    let mock = Routed::start(routes(&storage, vec![]));
    let path = write_bytes(&sb, "shot.png", PNG);

    let o = run(&sb, &mock, &["file", "upload", &path, "--json"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_no_leak(&o);
    let v = stdout_json(&o);
    assert_eq!(v["workspace"], "example");
    assert_eq!(v["assetUrl"], ASSET);
    assert_eq!(v["filename"], "shot.png");
    assert_eq!(v["contentType"], "image/png");
    assert_eq!(v["size"], 4);
    assert_eq!(v["markdown"], format!("![shot.png]({ASSET})"));
    assert_eq!(
        mock.of("FileUpload"),
        vec![json!({ "contentType": "image/png", "filename": "shot.png", "size": 4 })]
    );

    let hits = storage.hits();
    assert_eq!(hits.len(), 1, "{hits:?}");
    let put = &hits[0];
    assert_eq!(put.method, "PUT");
    assert_eq!(put.path, "/upload/signed?sig=abc");
    assert_eq!(put.body, PNG);
    assert_eq!(put.headers["content-type"], "image/png");
    assert_eq!(put.headers["x-goog-meta-test"], "yes");
    // The signed URL carries its own authorization: ours must not go there.
    assert!(!put.headers.contains_key("authorization"), "{put:?}");
}

#[test]
fn upload_text_shows_the_url_and_the_markdown_and_quiet_only_the_url() {
    let sb = workspace();
    let storage = Storage::start(vec![("/upload/signed?sig=abc", 200, vec![])]);
    let mock = Routed::start(routes(
        &storage,
        vec![(
            "FileUpload",
            vec![upload_reply(&storage, "a [1].pdf", "application/pdf", 4)],
        )],
    ));
    let path = write_bytes(&sb, "report.pdf", b"%PDF");

    let o = run(
        &sb,
        &mock,
        &["file", "upload", &path, "--name", "a [1].pdf"],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let text = stdout(&o);
    assert!(text.contains(&format!("URL:      {ASSET}")), "{text}");
    assert!(
        text.contains(&format!("Markdown: [a \\[1\\].pdf]({ASSET})")),
        "{text}"
    );
    assert_eq!(
        mock.of("FileUpload")[0],
        json!({ "contentType": "application/pdf", "filename": "a [1].pdf", "size": 4 })
    );

    let o = run(&sb, &mock, &["file", "upload", &path, "--quiet"]);
    assert_eq!(stdout(&o).trim(), ASSET);
}

#[test]
fn a_content_type_can_be_given() {
    let sb = workspace();
    let storage = Storage::start(vec![("/upload/signed?sig=abc", 200, vec![])]);
    let mock = Routed::start(routes(&storage, vec![]));
    let path = write_bytes(&sb, "dump", b"abcd");
    let o = run(
        &sb,
        &mock,
        &["file", "upload", &path, "--content-type", "text/plain"],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(mock.of("FileUpload")[0]["contentType"], "text/plain");
}

#[test]
fn a_file_that_is_too_large_empty_or_not_a_file_is_refused_before_any_request() {
    let sb = workspace();
    let storage = Storage::start(vec![]);
    let mock = Routed::start(routes(&storage, vec![]));
    let big = sb.cwd().join("big.bin");
    std::fs::File::create(&big)
        .unwrap()
        .set_len(25 * 1024 * 1024 + 1)
        .unwrap();
    let exact = sb.cwd().join("exact.bin");
    std::fs::File::create(&exact)
        .unwrap()
        .set_len(25 * 1024 * 1024)
        .unwrap();
    let empty = write_bytes(&sb, "empty.txt", b"");
    let dir = sb.cwd().to_string_lossy().into_owned();
    let missing = sb.cwd().join("nope.png").to_string_lossy().into_owned();

    for (path, wanted) in [
        (big.to_string_lossy().into_owned(), "limit is 25 MiB"),
        (empty, "empty"),
        (dir, "not a regular file"),
        (missing, "cannot read"),
    ] {
        for args in [
            vec!["file", "upload", &path],
            vec!["issue", "attach-file", "EX-23", &path],
        ] {
            let o = run(&sb, &mock, &args);
            assert_eq!(code(&o), 2, "{args:?}: {}", stderr(&o));
            assert!(stderr(&o).contains(wanted), "{args:?}: {}", stderr(&o));
        }
    }
    assert!(mock.ops().is_empty(), "{:?}", mock.ops());
    assert!(storage.hits().is_empty());

    // Exactly at the limit is fine for the checks (the mock then refuses the upload slot).
    let o = run(&sb, &mock, &["file", "upload", &exact.to_string_lossy()]);
    assert_ne!(code(&o), 2, "{}", stderr(&o));
    assert!(mock.ops().contains(&"FileUpload".to_owned()));
}

#[test]
fn a_refused_put_is_an_error_and_nothing_is_attached() {
    let sb = workspace();
    let storage = Storage::start(vec![("/upload/signed?sig=abc", 403, b"expired".to_vec())]);
    let mock = Routed::start(routes(&storage, vec![]));
    let path = write_bytes(&sb, "shot.png", PNG);

    let o = run(&sb, &mock, &["issue", "attach-file", "EX-23", &path]);
    assert_eq!(code(&o), 1, "{}", stderr(&o));
    let err = stderr(&o);
    assert!(
        err.contains("refused the upload (HTTP 403)") && err.contains("expired"),
        "{err}"
    );
    assert!(err.contains("nothing was attached"), "{err}");
    assert!(mock.of("AttachmentCreate").is_empty());
    // Nothing was stored, so there is nothing to clean up.
    assert!(mock.of("FileUploadDangerouslyDelete").is_empty());
}

#[test]
fn linear_giving_no_place_to_put_the_file_is_an_error() {
    let sb = workspace();
    let storage = Storage::start(vec![]);
    let mock = Routed::start(routes(
        &storage,
        vec![(
            "FileUpload",
            vec![data(
                json!({ "fileUpload": { "success": false, "uploadFile": null } }),
            )],
        )],
    ));
    let path = write_bytes(&sb, "shot.png", PNG);
    let o = run(&sb, &mock, &["file", "upload", &path]);
    assert_eq!(code(&o), 1, "{}", stderr(&o));
    assert!(storage.hits().is_empty());
}

// ------------------------------------------------------------------ issue attach-file

#[test]
fn attach_file_uploads_then_attaches_the_asset_url_to_the_issue() {
    let sb = workspace();
    let storage = Storage::start(vec![("/upload/signed?sig=abc", 200, vec![])]);
    let mock = Routed::start(routes(&storage, vec![]));
    let path = write_bytes(&sb, "shot.png", PNG);

    let o = run(
        &sb,
        &mock,
        &["issue", "attach-file", "EX-23", &path, "--json"],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_no_leak(&o);
    let v = stdout_json(&o);
    assert_eq!(v["issue"], "EX-23");
    assert_eq!(v["attachmentId"], ATTACHMENT);
    assert_eq!(v["url"], ASSET);
    assert_eq!(v["markdown"], format!("![shot.png]({ASSET})"));
    assert_eq!(
        mock.of("AttachmentCreate"),
        vec![json!({ "input": {
            "issueId": "id-EX-23", "url": ASSET, "title": "shot.png",
            "subtitle": "image/png, 4 B"
        } })]
    );
    // Upload first, attach last.
    let ops = mock.ops();
    let at = |op: &str| ops.iter().position(|o| o == op).unwrap();
    assert!(at("IssueWriteView") < at("FileUpload") && at("FileUpload") < at("AttachmentCreate"));
    assert_eq!(storage.hits().len(), 1);
    assert!(mock.of("FileUploadDangerouslyDelete").is_empty());
}

#[test]
fn attach_file_text_and_title() {
    let sb = workspace();
    let storage = Storage::start(vec![("/upload/signed?sig=abc", 200, vec![])]);
    let mock = Routed::start(routes(&storage, vec![]));
    let path = write_bytes(&sb, "shot.png", PNG);

    let o = run(
        &sb,
        &mock,
        &[
            "issue",
            "attach-file",
            "EX-23",
            &path,
            "--title",
            "Login bug",
        ],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(
        mock.of("AttachmentCreate")[0]["input"]["title"],
        "Login bug"
    );
    // The title shown is what Linear answered with.
    assert_eq!(
        stdout(&o).trim(),
        format!("EX-23  shot.png  {ASSET}  (attached, 4 B)")
    );
    let o = run(
        &sb,
        &mock,
        &["issue", "attach-file", "EX-23", &path, "--quiet"],
    );
    assert_eq!(stdout(&o).trim(), ASSET);
}

#[test]
fn attach_file_follows_ownership_and_uploads_nothing_when_refused() {
    let sb = workspace();
    let storage = Storage::start(vec![]);
    let mut r = routes(&storage, vec![]);
    r.retain(|(o, _)| *o != "IssueWriteView");
    r.push((
        "IssueWriteView",
        vec![view("EX-23")
            .assigned_to(Some(BOT))
            .in_project(PROJECT, Some(BOT))
            .reply()],
    ));
    let mock = Routed::start(r);
    let path = write_bytes(&sb, "shot.png", PNG);
    let o = run(&sb, &mock, &["issue", "attach-file", "EX-23", &path]);
    assert_eq!(code(&o), 4, "{}", stderr(&o));
    assert!(!mock.ops().contains(&"FileUpload".to_owned()));
    assert!(storage.hits().is_empty());
}

fn failing_attach() -> Vec<Reply> {
    vec![graphql_error("attachment refused")]
}

#[test]
fn a_failed_attach_deletes_the_file_it_stored() {
    let sb = workspace();
    let storage = Storage::start(vec![("/upload/signed?sig=abc", 200, vec![])]);
    let mock = Routed::start(routes(
        &storage,
        vec![("AttachmentCreate", failing_attach())],
    ));
    let path = write_bytes(&sb, "shot.png", PNG);

    let o = run(&sb, &mock, &["issue", "attach-file", "EX-23", &path]);
    assert_eq!(code(&o), 1, "{}", stderr(&o));
    let err = stderr(&o);
    assert!(
        err.contains("attachment refused") && err.contains("the uploaded file was deleted again"),
        "{err}"
    );
    // Attaching is an upsert on the URL, so it is tried again before giving up.
    assert_eq!(mock.of("AttachmentCreate").len(), 3);
    assert_eq!(
        mock.of("AttachmentTargetsQuery"),
        vec![json!({ "url": ASSET })]
    );
    assert_eq!(
        mock.of("FileUploadDangerouslyDelete"),
        vec![json!({ "assetUrl": ASSET })]
    );
}

#[test]
fn a_failed_attach_keeps_the_file_when_an_attachment_uses_it() {
    let sb = workspace();
    let storage = Storage::start(vec![("/upload/signed?sig=abc", 200, vec![])]);
    let mock = Routed::start(routes(
        &storage,
        vec![
            ("AttachmentCreate", failing_attach()),
            (
                "AttachmentTargetsQuery",
                vec![attachment_targets(&[(ATTACHMENT, ASSET, "id-EX-23")])],
            ),
        ],
    ));
    let path = write_bytes(&sb, "shot.png", PNG);
    let o = run(&sb, &mock, &["issue", "attach-file", "EX-23", &path]);
    assert_eq!(code(&o), 1, "{}", stderr(&o));
    let err = stderr(&o);
    assert!(
        err.contains("exists after all") && err.contains(ASSET),
        "{err}"
    );
    assert!(mock.of("FileUploadDangerouslyDelete").is_empty());
}

#[test]
fn a_failed_attach_keeps_the_file_when_it_cannot_be_checked() {
    let sb = workspace();
    let storage = Storage::start(vec![("/upload/signed?sig=abc", 200, vec![])]);
    let mock = Routed::start(routes(
        &storage,
        vec![
            ("AttachmentCreate", failing_attach()),
            ("AttachmentTargetsQuery", vec![graphql_error("down")]),
        ],
    ));
    let path = write_bytes(&sb, "shot.png", PNG);
    let o = run(&sb, &mock, &["issue", "attach-file", "EX-23", &path]);
    assert_eq!(code(&o), 1, "{}", stderr(&o));
    assert!(stderr(&o).contains("so it was kept"), "{}", stderr(&o));
    assert!(mock.of("FileUploadDangerouslyDelete").is_empty());
}

#[test]
fn a_file_that_cannot_be_deleted_is_named() {
    let sb = workspace();
    let storage = Storage::start(vec![("/upload/signed?sig=abc", 200, vec![])]);
    let mock = Routed::start(routes(
        &storage,
        vec![
            ("AttachmentCreate", failing_attach()),
            (
                "FileUploadDangerouslyDelete",
                vec![graphql_error("not allowed")],
            ),
        ],
    ));
    let path = write_bytes(&sb, "shot.png", PNG);
    let o = run(&sb, &mock, &["issue", "attach-file", "EX-23", &path]);
    assert_eq!(code(&o), 1, "{}", stderr(&o));
    let err = stderr(&o);
    assert!(
        err.contains("COULD NOT delete the uploaded file") && err.contains(ASSET),
        "{err}"
    );
}

// ------------------------------------------------------------------ file download

fn download(sb: &Sandbox, api: &Storage, args: &[&str]) -> std::process::Output {
    let mut full = vec!["file", "download"];
    full.extend_from_slice(args);
    sb.run_url(
        &full,
        &format!("{}/graphql", api.base),
        &[("LINEAR_API_KEY_EXAMPLE", KEY)],
    )
}

#[test]
fn download_sends_the_credential_to_the_api_origin_and_saves_the_file() {
    let sb = workspace();
    let storage = Storage::start(vec![("/files/shot.png", 200, PNG.to_vec())]);
    let o = download(&sb, &storage, &[&storage.url("/files/shot.png"), "--json"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_no_leak(&o);
    let v = stdout_json(&o);
    assert_eq!(v["path"], "shot.png");
    assert_eq!(v["size"], 4);
    assert_eq!(std::fs::read(sb.cwd().join("shot.png")).unwrap(), PNG);
    let hits = storage.hits();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].method, "GET");
    assert_eq!(hits[0].headers["authorization"], KEY);
    // No partial file is left behind.
    let names: Vec<_> = std::fs::read_dir(sb.cwd())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, vec!["shot.png".to_owned()]);
}

#[test]
fn download_sends_no_credential_to_another_host() {
    let sb = workspace();
    let api = Storage::start(vec![]);
    let elsewhere = Storage::start(vec![("/files/shot.png", 200, PNG.to_vec())]);
    let o = download(
        &sb,
        &api,
        &[&elsewhere.url("/files/shot.png"), "-o", "copy.png"],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(std::fs::read(sb.cwd().join("copy.png")).unwrap(), PNG);
    let hits = elsewhere.hits();
    assert_eq!(hits.len(), 1);
    assert!(!hits[0].headers.contains_key("authorization"), "{hits:?}");
    assert!(api.hits().is_empty());
}

#[test]
fn download_will_not_replace_a_file_without_force() {
    let sb = workspace();
    let storage = Storage::start(vec![("/files/shot.png", 200, PNG.to_vec())]);
    std::fs::write(sb.cwd().join("shot.png"), b"mine").unwrap();
    let url = storage.url("/files/shot.png");

    let o = download(&sb, &storage, &[&url]);
    assert_eq!(code(&o), 2, "{}", stderr(&o));
    assert!(stderr(&o).contains("--force"), "{}", stderr(&o));
    assert_eq!(std::fs::read(sb.cwd().join("shot.png")).unwrap(), b"mine");
    assert!(storage.hits().is_empty());

    let o = download(&sb, &storage, &[&url, "--force", "--quiet"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(stdout(&o).trim(), "shot.png");
    assert_eq!(std::fs::read(sb.cwd().join("shot.png")).unwrap(), PNG);
}

#[test]
fn download_to_standard_output() {
    let sb = workspace();
    let storage = Storage::start(vec![("/files/shot.png", 200, PNG.to_vec())]);
    let o = download(&sb, &storage, &[&storage.url("/files/shot.png"), "-o", "-"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(o.stdout, PNG);
}

#[test]
fn a_failed_download_leaves_nothing_and_says_why() {
    let sb = workspace();
    let storage = Storage::start(vec![
        ("/files/forbidden", 403, b"no".to_vec()),
        ("/files/broken", 500, b"oops".to_vec()),
    ]);
    for (path, wanted, exit) in [
        ("/files/missing", "no such file", 1),
        ("/files/forbidden", "refused the download", 3),
        ("/files/broken", "HTTP 500", 1),
    ] {
        let o = download(&sb, &storage, &[&storage.url(path)]);
        assert_eq!(code(&o), exit, "{path}: {}", stderr(&o));
        assert!(stderr(&o).contains(wanted), "{path}: {}", stderr(&o));
    }
    assert_eq!(std::fs::read_dir(sb.cwd()).unwrap().count(), 0);
}

#[test]
fn download_needs_a_name_and_an_http_url() {
    let sb = workspace();
    let storage = Storage::start(vec![]);
    let o = download(&sb, &storage, &[&storage.base]);
    assert_eq!(code(&o), 2, "{}", stderr(&o));
    assert!(stderr(&o).contains("--output"), "{}", stderr(&o));
    let o = download(&sb, &storage, &["file:///etc/passwd", "-o", "x"]);
    assert_eq!(code(&o), 2, "{}", stderr(&o));
    let o = download(&sb, &storage, &["not a url", "-o", "x"]);
    assert_eq!(code(&o), 2, "{}", stderr(&o));
    assert!(storage.hits().is_empty());
}
