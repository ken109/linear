//! `linear file upload|download` (the upload lives in `write::file`).
//!
//! `upload` puts a file in the workspace's file storage and prints the URL and the markdown that
//! embeds it. `download` saves a file from a Linear URL, sending the workspace's credential
//! where Linear needs it.

use super::{write, Ctx};
use crate::error::{CliError, Result};
use clap::{Args, Subcommand};
use linear_core::files::{filename_from_url, MAX_DOWNLOAD_BYTES};
use serde::Serialize;
use std::io::Write as _;
use std::path::{Path, PathBuf};

#[derive(Debug, Subcommand)]
pub enum FileCommand {
    /// Show how to use these commands, briefly, for an AI agent
    Usage,
    /// Upload a file to Linear and print its URL and the markdown that embeds it
    ///
    /// The file is stored in the workspace and visible only to people who can sign in to it.
    /// Paste the markdown (`![name](url)` for an image, `[name](url)` for anything else) into a
    /// description or comment to show it inline; `issue attach-file` attaches a file to an issue
    /// instead. At most 25 MiB, so a screenshot, a log or a document and not a video; a larger,
    /// empty or unreadable file is refused before anything is sent. The `--timeout` limit
    /// applies to each request, so a large file on a slow line needs it raised.
    Upload(write::file::UploadCmd),
    /// Save a file from a Linear URL (such as the URL `file upload` printed)
    ///
    /// Sends the credential of the workspace, but only to Linear's file host
    /// (uploads.linear.app) and never on to another host a redirect leads to, so a URL of any
    /// other site is fetched without it. Refuses to replace a file unless --force, and to
    /// save more than 100 MiB; a download that fails leaves no partial file.
    Download(DownloadCmd),
}

#[derive(Debug, Args)]
pub struct DownloadCmd {
    /// The file's URL
    pub url: String,
    /// Where to save it (`-` for standard output) [default: the last part of the URL, in the
    /// current directory]
    #[arg(short, long, value_name = "PATH")]
    pub output: Option<PathBuf>,
    /// Replace the file if it exists
    #[arg(long)]
    pub force: bool,
}

pub fn run(ctx: &Ctx, cmd: &FileCommand) -> Result<()> {
    match cmd {
        FileCommand::Usage => unreachable!("handled before the context is built"),
        FileCommand::Upload(args) => write::file::upload(ctx, args),
        FileCommand::Download(args) => download(ctx, args),
    }
}

/// What `download` prints.
#[derive(Serialize)]
struct Downloaded<'a> {
    workspace: &'a str,
    url: &'a str,
    /// Where it was saved (`-`: standard output).
    path: String,
    size: u64,
}

fn download(ctx: &Ctx, cmd: &DownloadCmd) -> Result<()> {
    let to_stdout = cmd.output.as_deref() == Some(Path::new("-"));
    let path = match &cmd.output {
        Some(p) => p.clone(),
        None => PathBuf::from(filename_from_url(&cmd.url).ok_or_else(|| {
            CliError::usage("the URL does not end in a file name; pass --output <PATH>")
        })?),
    };
    if !to_stdout && path.exists() && !cmd.force {
        return Err(CliError::usage(format!(
            "{} exists; pass --force to replace it, or --output to save elsewhere",
            path.display()
        )));
    }

    let session = ctx.session()?;
    let size = if to_stdout {
        let mut out = std::io::stdout().lock();
        let n = session
            .client
            .download(&cmd.url, MAX_DOWNLOAD_BYTES, &mut out)?;
        out.flush()?;
        n
    } else {
        // Written next to the target and renamed into place, so a download that fails
        // never leaves a partial file under the real name.
        let part = {
            let mut name = path.file_name().unwrap_or_default().to_os_string();
            name.push(format!(".{}.part", std::process::id()));
            path.with_file_name(name)
        };
        let result = (|| -> Result<u64> {
            let mut file = std::fs::File::create(&part)
                .map_err(|e| CliError::general(format!("cannot write {}: {e}", part.display())))?;
            let n = session
                .client
                .download(&cmd.url, MAX_DOWNLOAD_BYTES, &mut file)?;
            file.flush()?;
            drop(file);
            std::fs::rename(&part, &path)
                .map_err(|e| CliError::general(format!("cannot write {}: {e}", path.display())))?;
            Ok(n)
        })();
        match result {
            Ok(n) => n,
            Err(e) => {
                let _ = std::fs::remove_file(&part);
                return Err(e);
            }
        }
    };

    if to_stdout {
        // The file is on standard output; say nothing there.
        ctx.out
            .status(&format!("saved {size} bytes to standard output"));
        return Ok(());
    }
    let shown = path.display().to_string();
    let value = Downloaded {
        workspace: &session.workspace,
        url: &cmd.url,
        path: shown.clone(),
        size,
    };
    ctx.out.emit(
        &value,
        || format!("{shown}  ({})", write::file::human_size(size)),
        || shown.clone(),
    );
    Ok(())
}
