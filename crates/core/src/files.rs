//! File uploads: the operations that ask Linear for a place to put a file, the
//! limits the CLI holds itself to, and the text that embeds a file in markdown.
//!
//! Linear takes a file in two steps. `fileUpload` returns a signed URL, the
//! headers the upload must carry and the permanent asset URL; the bytes are then
//! sent with a PUT to the signed URL (that is the CLI's job: this crate does no
//! I/O). The asset URL is what an attachment (`attachmentCreate`) or an image
//! in a description or comment points at.

use crate::schema;
use cynic::{MutationBuilder, Operation};

/// The largest file `file upload` and `issue attach-file` send: 25 MiB.
///
/// A screenshot, a log or a PDF fits; a video or a disk image does not. The
/// whole file is read into memory and sent in one request, which is also why
/// the limit is kept low. Linear enforces limits of its own (by plan) on top.
pub const MAX_UPLOAD_BYTES: u64 = 25 * 1024 * 1024;

/// The largest file `file download` saves: 100 MiB. It is streamed to disk, so
/// the limit protects the disk and a mistyped URL, not memory.
pub const MAX_DOWNLOAD_BYTES: u64 = 100 * 1024 * 1024;

/// Why a file of `size` bytes is not sent, or `None` when it is fine.
pub fn upload_size_problem(size: u64) -> Option<String> {
    if size == 0 {
        return Some("the file is empty".to_owned());
    }
    if size > MAX_UPLOAD_BYTES {
        return Some(format!(
            "the file is {} MiB; the limit is {} MiB (a screenshot or a document, not a video)",
            size.div_ceil(1024 * 1024),
            MAX_UPLOAD_BYTES / (1024 * 1024)
        ));
    }
    None
}

/// The MIME type to send for a file name, from its extension (ignoring case).
/// Anything not listed is `application/octet-stream`.
pub fn content_type_for(filename: &str) -> &'static str {
    let ext = filename
        .rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "bmp" => "image/bmp",
        "heic" => "image/heic",
        "pdf" => "application/pdf",
        "txt" | "log" => "text/plain",
        "md" => "text/markdown",
        "csv" => "text/csv",
        "html" | "htm" => "text/html",
        "json" => "application/json",
        "zip" => "application/zip",
        "gz" => "application/gzip",
        "mp4" => "video/mp4",
        "mov" => "video/quicktime",
        "webm" => "video/webm",
        _ => "application/octet-stream",
    }
}

/// The markdown that puts an uploaded file in a description or comment: an
/// image for an image type, a link for everything else.
pub fn embed_markdown(filename: &str, content_type: &str, asset_url: &str) -> String {
    let label = filename
        .replace('\\', "\\\\")
        .replace('[', "\\[")
        .replace(']', "\\]");
    if content_type.starts_with("image/") {
        format!("![{label}]({asset_url})")
    } else {
        format!("[{label}]({asset_url})")
    }
}

/// The last path segment of a URL, as a file name: the query and the fragment
/// are dropped, `%20` and the like are decoded, and a segment that would leave
/// the directory (`..`, a path separator) is refused. `None` when nothing usable is left.
pub fn filename_from_url(url: &str) -> Option<String> {
    let path = url.split(['?', '#']).next()?;
    let path = path.split_once("://").map_or(path, |(_, rest)| rest);
    let (_, path) = path.split_once('/')?;
    let segment = path.rsplit('/').find(|s| !s.is_empty())?;
    let name = percent_decode(segment);
    let unsafe_name = name == "."
        || name == ".."
        || name.contains(['/', '\\', '\0'])
        || name.chars().any(char::is_control);
    (!unsafe_name).then_some(name)
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Some(v) = s
                .get(i + 1..i + 3)
                .and_then(|h| u8::from_str_radix(h, 16).ok())
            {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

// ---------------------------------------------------------------- fileUpload

#[derive(cynic::QueryVariables, Debug, Clone)]
pub struct FileUploadVars {
    pub content_type: String,
    pub filename: String,
    pub size: i32,
}

/// Asks Linear for a place to put a file (`fileUpload`; the file is private to
/// the workspace: `makePublic` is left at its default).
#[derive(cynic::QueryFragment, Debug, Clone, PartialEq)]
#[cynic(graphql_type = "Mutation", variables = "FileUploadVars")]
pub struct FileUpload {
    #[arguments(contentType: $content_type, filename: $filename, size: $size)]
    pub file_upload: UploadResult,
}

#[derive(cynic::QueryFragment, Debug, Clone, PartialEq)]
#[cynic(graphql_type = "UploadPayload")]
pub struct UploadResult {
    pub success: bool,
    pub upload_file: Option<UploadTarget>,
}

/// Where and how to send a file, and where it will be afterwards.
#[derive(cynic::QueryFragment, Debug, Clone, PartialEq)]
#[cynic(graphql_type = "UploadFile")]
pub struct UploadTarget {
    pub asset_url: String,
    pub content_type: String,
    pub filename: String,
    pub size: i32,
    pub upload_url: String,
    pub headers: Vec<UploadHeader>,
}

#[derive(cynic::QueryFragment, Debug, Clone, PartialEq)]
#[cynic(graphql_type = "UploadFileHeader")]
pub struct UploadHeader {
    pub key: String,
    pub value: String,
}

pub fn file_upload(
    filename: impl Into<String>,
    content_type: impl Into<String>,
    size: i32,
) -> Operation<FileUpload, FileUploadVars> {
    FileUpload::build(FileUploadVars {
        content_type: content_type.into(),
        filename: filename.into(),
        size,
    })
}

// ---------------------------------------------------------------- fileUploadDangerouslyDelete

#[derive(cynic::QueryVariables, Debug, Clone)]
pub struct FileUploadDeleteVars {
    pub asset_url: String,
}

#[derive(cynic::QueryFragment, Debug, Clone, PartialEq)]
#[cynic(graphql_type = "Mutation", variables = "FileUploadDeleteVars")]
pub struct FileUploadDangerouslyDelete {
    #[arguments(assetUrl: $asset_url)]
    pub file_upload_dangerously_delete: FileUploadDeleteResult,
}

#[derive(cynic::QueryFragment, Debug, Clone, PartialEq)]
#[cynic(graphql_type = "FileUploadDeletePayload")]
pub struct FileUploadDeleteResult {
    pub success: bool,
}

/// Deletes an uploaded file by its asset URL. Linear marks this internal and
/// "a last resort": it breaks whatever references the file. The CLI uses it for
/// one thing only, to remove a file it uploaded a moment ago when attaching it
/// failed, and only after checking that nothing references it.
pub fn file_upload_delete(
    asset_url: impl Into<String>,
) -> Operation<FileUploadDangerouslyDelete, FileUploadDeleteVars> {
    FileUploadDangerouslyDelete::build(FileUploadDeleteVars {
        asset_url: asset_url.into(),
    })
}
