//! `linear file upload` and `linear issue attach-file`.
//!
//! A file goes to Linear in two steps: `fileUpload` returns a signed URL and
//! the headers to send, then the bytes are PUT to it. The result is an asset
//! URL that stays private to the workspace. `file upload` stops there and
//! prints the URL and the markdown that embeds it (for a description or a
//! comment). `issue attach-file` goes on and attaches the URL to an issue.
//!
//! Limits and failures:
//!
//! * A file larger than `MAX_UPLOAD_BYTES` (25 MiB), an empty one and a path
//!   that is not a regular file are refused before any request.
//! * If the PUT fails, nothing was stored and nothing needs cleaning up.
//! * If the file was stored but attaching it fails (after trying again:
//!   `attachmentCreate` is an upsert on the URL, so repeating it is safe), the
//!   stored file is deleted. The exception is when Linear turns out to hold an
//!   attachment with that URL, or that cannot be checked: then the file is kept
//!   and its URL is named in the error. A file that could not be deleted is
//!   named as well.

use super::issue::{fetch_issue, placement_of};
use super::{retry, WriteSession, ATTACH_WAITS};
use crate::commands::Ctx;
use crate::error::{CliError, Result};
use clap::Args;
use linear_core::files::{
    self, content_type_for, embed_markdown, upload_size_problem, UploadTarget,
};
use linear_core::guard::Write;
use linear_core::inputs::{self, AttachmentCreate, AttachmentCreateInput};
use linear_core::read;
use linear_core::rules::{Draft, Operation};
use serde::Serialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Args)]
pub struct UploadCmd {
    /// The file to upload (at most 25 MiB)
    pub path: PathBuf,
    /// The name Linear shows for the file [default: the file's own name]
    #[arg(long, value_name = "NAME")]
    pub name: Option<String>,
    /// The MIME type [default: from the file's extension]
    #[arg(long, value_name = "TYPE")]
    pub content_type: Option<String>,
}

#[derive(Debug, Args)]
pub struct AttachFileCmd {
    /// Issue identifier (such as KK-12) or id
    pub issue: String,
    /// The file to attach (at most 25 MiB)
    pub path: PathBuf,
    /// The attachment's title [default: the file's name]
    #[arg(long, value_name = "TEXT")]
    pub title: Option<String>,
    /// The name Linear shows for the file [default: the file's own name]
    #[arg(long, value_name = "NAME")]
    pub name: Option<String>,
    /// The MIME type [default: from the file's extension]
    #[arg(long, value_name = "TYPE")]
    pub content_type: Option<String>,
}

/// A file that has been read and checked, ready to send.
struct Local {
    name: String,
    content_type: String,
    bytes: Vec<u8>,
}

/// Read `path` after checking what can be checked without a request.
fn read_local(path: &Path, name: Option<&str>, content_type: Option<&str>) -> Result<Local> {
    let meta = std::fs::metadata(path)
        .map_err(|e| CliError::usage(format!("cannot read {}: {e}", path.display())))?;
    if !meta.is_file() {
        return Err(CliError::usage(format!(
            "{} is not a regular file",
            path.display()
        )));
    }
    if let Some(problem) = upload_size_problem(meta.len()) {
        return Err(CliError::usage(format!("{}: {problem}", path.display())));
    }
    let name = match name.map(str::trim) {
        Some("") => return Err(CliError::usage("--name must not be empty")),
        Some(n) => n.to_owned(),
        None => path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .ok_or_else(|| CliError::usage(format!("{} has no file name", path.display())))?,
    };
    let content_type = match content_type.map(str::trim) {
        Some("") => return Err(CliError::usage("--content-type must not be empty")),
        Some(t) => t.to_owned(),
        None => content_type_for(&name).to_owned(),
    };
    let bytes = std::fs::read(path)
        .map_err(|e| CliError::usage(format!("cannot read {}: {e}", path.display())))?;
    // The file may have changed between the check and the read.
    if let Some(problem) = upload_size_problem(bytes.len() as u64) {
        return Err(CliError::usage(format!("{}: {problem}", path.display())));
    }
    Ok(Local {
        name,
        content_type,
        bytes,
    })
}

/// Ask Linear for a place to put the file and send it there.
fn store(ws: &WriteSession, file: &Local) -> Result<UploadTarget> {
    let size = i32::try_from(file.bytes.len())
        .map_err(|_| CliError::usage("the file is too large to upload"))?;
    let data: files::FileUpload =
        ws.client
            .execute(&files::file_upload(&file.name, &file.content_type, size))?;
    let target = match data.file_upload {
        files::UploadResult {
            success: true,
            upload_file: Some(target),
        } => target,
        _ => {
            return Err(CliError::general(
                "Linear could not prepare the upload (it gave no place to put the file)",
            ))
        }
    };
    let mut headers: Vec<(String, String)> = target
        .headers
        .iter()
        .map(|h| (h.key.clone(), h.value.clone()))
        .collect();
    if !headers
        .iter()
        .any(|(k, _)| k.eq_ignore_ascii_case("content-type"))
    {
        headers.push(("Content-Type".to_owned(), file.content_type.clone()));
    }
    ws.client
        .put_file(&target.upload_url, &headers, &file.bytes)
        .map_err(|e| CliError::new(e.code, format!("{}; nothing was attached", e.message)))?;
    Ok(target)
}

pub fn human_size(bytes: u64) -> String {
    const KIB: f64 = 1024.0;
    let b = bytes as f64;
    if b < KIB {
        format!("{bytes} B")
    } else if b < KIB * KIB {
        format!("{:.1} KiB", b / KIB)
    } else {
        format!("{:.1} MiB", b / (KIB * KIB))
    }
}

// ---------------------------------------------------------------- file upload

/// What `file upload` prints.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Uploaded<'a> {
    workspace: &'a str,
    /// Where the file is now. Private to the workspace: only people who can sign in see it.
    asset_url: &'a str,
    filename: &'a str,
    content_type: &'a str,
    size: usize,
    /// Markdown that embeds the file in a description or comment.
    markdown: String,
}

pub fn upload(ctx: &Ctx, cmd: &UploadCmd) -> Result<()> {
    let file = read_local(&cmd.path, cmd.name.as_deref(), cmd.content_type.as_deref())?;
    let ws = ctx.write_session()?;
    let target = store(&ws, &file)?;
    let markdown = embed_markdown(&file.name, &file.content_type, &target.asset_url);
    let value = Uploaded {
        workspace: &ws.workspace,
        asset_url: &target.asset_url,
        filename: &file.name,
        content_type: &file.content_type,
        size: file.bytes.len(),
        markdown: markdown.clone(),
    };
    ctx.out.emit(
        &value,
        || {
            format!(
                "{}  ({}, {})\nURL:      {}\nMarkdown: {}",
                file.name,
                file.content_type,
                human_size(file.bytes.len() as u64),
                target.asset_url,
                markdown
            )
        },
        || target.asset_url.clone(),
    );
    Ok(())
}

// ---------------------------------------------------------------- issue attach-file

/// What `attach-file` prints.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Attached<'a> {
    workspace: &'a str,
    issue: &'a str,
    attachment_id: &'a str,
    /// The attachment's URL: where the file is.
    url: &'a str,
    title: &'a str,
    filename: &'a str,
    content_type: &'a str,
    size: usize,
    markdown: String,
}

pub fn attach_file(ctx: &Ctx, cmd: &AttachFileCmd) -> Result<()> {
    let file = read_local(&cmd.path, cmd.name.as_deref(), cmd.content_type.as_deref())?;
    let title = match cmd.title.as_deref().map(str::trim) {
        Some("") => return Err(CliError::usage("--title must not be empty")),
        Some(t) => t.to_owned(),
        None => file.name.clone(),
    };

    let ws = ctx.write_session()?;
    let view = fetch_issue(&ws, &cmd.issue)?;
    // Attaching is a write to the issue: it follows the same ownership as changing it.
    ws.guard(
        &Write::update_issue(&view.issue, placement_of(&view)),
        false,
    )?;
    ws.validate(&Draft::new(Operation::IssueUpdate))?;
    let identifier = view.issue.identifier.as_str();

    let target = store(&ws, &file)?;
    let input = AttachmentCreateInput {
        issue_id: view.issue.id.inner().to_owned(),
        url: target.asset_url.clone(),
        title: title.clone(),
        subtitle: Some(format!(
            "{}, {}",
            file.content_type,
            human_size(file.bytes.len() as u64)
        )),
        metadata: None,
    };
    let attached = retry(ws.out, "attaching the file", &ATTACH_WAITS, || {
        let data: AttachmentCreate = ws
            .client
            .execute(&inputs::attachment_create(input.clone()))?;
        if data.attachment_create.success {
            Ok(data.attachment_create.attachment)
        } else {
            Err(CliError::general(format!(
                "Linear could not attach the file to {identifier}"
            )))
        }
    });
    let attachment = match attached {
        Ok(a) => a,
        Err(cause) => return Err(clean_up(&ws, &target.asset_url, cause)),
    };

    let markdown = embed_markdown(&file.name, &file.content_type, &target.asset_url);
    let value = Attached {
        workspace: &ws.workspace,
        issue: identifier,
        attachment_id: attachment.id.inner(),
        url: &attachment.url,
        title: &attachment.title,
        filename: &file.name,
        content_type: &file.content_type,
        size: file.bytes.len(),
        markdown,
    };
    ctx.out.emit(
        &value,
        || {
            format!(
                "{identifier}  {}  {}  (attached, {})",
                attachment.title,
                attachment.url,
                human_size(file.bytes.len() as u64)
            )
        },
        || attachment.url.clone(),
    );
    Ok(())
}

/// The file was stored but could not be attached. Delete it, unless something
/// may reference it: it is brand new, so normally nothing does, but a lost
/// response can hide a `attachmentCreate` that did go through, and deleting the
/// file under a live attachment would break it.
fn clean_up(ws: &WriteSession, asset_url: &str, cause: CliError) -> CliError {
    let found: Result<read::AttachmentTargetsQuery> =
        ws.client.execute(&read::attachment_targets(asset_url));
    let note = match found {
        Ok(f) if f.attachments_for_url.is_empty() => {
            let deleted: Result<files::FileUploadDangerouslyDelete> =
                ws.client.execute(&files::file_upload_delete(asset_url));
            match deleted {
                Ok(d) if d.file_upload_dangerously_delete.success => {
                    "the uploaded file was deleted again".to_owned()
                }
                Ok(_) => format!(
                    "COULD NOT delete the uploaded file, which nothing references: {asset_url}"
                ),
                Err(e) => format!(
                    "COULD NOT delete the uploaded file, which nothing references ({}): {asset_url}",
                    e.message
                ),
            }
        }
        Ok(_) => format!(
            "an attachment with that URL exists after all, so the uploaded file was kept: {asset_url}"
        ),
        Err(e) => format!(
            "could not check whether the uploaded file is attached ({}), so it was kept: {asset_url}",
            e.message
        ),
    };
    CliError::new(cause.code, format!("{}; {note}", cause.message))
}
