//! The limits, the content types and the markdown of file uploads.

use linear_core::files::*;
use linear_core::wire::build_request;
use serde_json::json;

#[test]
fn the_size_limit_is_explicit() {
    assert!(upload_size_problem(1).is_none());
    assert!(upload_size_problem(MAX_UPLOAD_BYTES).is_none());
    assert!(upload_size_problem(0).unwrap().contains("empty"));
    let big = upload_size_problem(MAX_UPLOAD_BYTES + 1).unwrap();
    assert!(
        big.contains("26 MiB") && big.contains("limit is 25 MiB"),
        "{big}"
    );
}

#[test]
fn a_content_type_comes_from_the_extension() {
    assert_eq!(content_type_for("shot.PNG"), "image/png");
    assert_eq!(content_type_for("a.b.jpeg"), "image/jpeg");
    assert_eq!(content_type_for("report.pdf"), "application/pdf");
    assert_eq!(content_type_for("noext"), "application/octet-stream");
    assert_eq!(content_type_for("x.weird"), "application/octet-stream");
}

#[test]
fn an_image_embeds_as_an_image_and_anything_else_as_a_link() {
    assert_eq!(
        embed_markdown("shot.png", "image/png", "https://uploads.linear.app/a/b/c"),
        "![shot.png](https://uploads.linear.app/a/b/c)"
    );
    assert_eq!(
        embed_markdown(
            "a [1].pdf",
            "application/pdf",
            "https://uploads.linear.app/a/b/c"
        ),
        "[a \\[1\\].pdf](https://uploads.linear.app/a/b/c)"
    );
}

#[test]
fn a_file_name_is_the_last_segment_of_the_url() {
    assert_eq!(
        filename_from_url("https://uploads.linear.app/org/id/shot%20one.png?x=1#f").as_deref(),
        Some("shot one.png")
    );
    assert_eq!(
        filename_from_url("https://uploads.linear.app/org/id/uuid/").as_deref(),
        Some("uuid")
    );
    // Nothing that leaves the directory.
    assert_eq!(filename_from_url("https://h/a/%2E%2E"), None);
    assert_eq!(filename_from_url("https://h/a/..%2Fx"), None);
    assert_eq!(filename_from_url("https://h/a/x%5Cy"), None);
    assert_eq!(filename_from_url("https://h"), None);
    assert_eq!(filename_from_url("https://h/%zz").as_deref(), Some("%zz"));
}

#[test]
fn the_upload_request_names_the_file_its_type_and_size() {
    let request = build_request(&file_upload("shot.png", "image/png", 1234));
    assert!(request.query.contains("fileUpload"));
    assert_eq!(
        request.variables,
        json!({ "contentType": "image/png", "filename": "shot.png", "size": 1234 })
    );
}

#[test]
fn the_cleanup_request_names_the_asset() {
    let request = build_request(&file_upload_delete("https://uploads.linear.app/a/b/c"));
    assert!(request.query.contains("fileUploadDangerouslyDelete"));
    assert_eq!(
        request.variables,
        json!({ "assetUrl": "https://uploads.linear.app/a/b/c" })
    );
}
