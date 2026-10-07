//! Moving the bytes of a file: the signed upload and the authorized download.
//!
//! Neither is a GraphQL request, so they live apart from `execute`. Neither is
//! sent again when it fails: an upload that did not finish attached nothing, and
//! running the command again starts over; a download is written to a temporary
//! file by the caller, which removes it on failure.
//!
//! Credentials go only where they belong. The signed upload URL carries its own
//! authorization and gets none of ours. A download gets the workspace's
//! credential only from Linear's file host (or the API endpoint's own origin,
//! which is how a proxy or a test double stands in for Linear), and a redirect
//! away from that host drops it.

use super::{short, Client};
use crate::error::{CliError, Result};
use std::io::{Read, Write};
use ureq::http::Uri;

/// The host Linear serves uploaded files from.
const FILE_HOST: &str = "uploads.linear.app";

impl Client {
    /// PUT `body` to a signed upload URL, with exactly the headers Linear named.
    pub fn put_file(&self, url: &str, headers: &[(String, String)], body: &[u8]) -> Result<()> {
        self.refuse_write("a file was about to be uploaded")?;
        let mut request = self.agent.put(url);
        for (key, value) in headers {
            request = request.header(key.as_str(), value.as_str());
        }
        let mut response = request.send(body).map_err(|e| {
            CliError::general(format!(
                "the upload to Linear's file storage failed: {}",
                short(&e)
            ))
        })?;
        let status = response.status().as_u16();
        if (200..300).contains(&status) {
            return Ok(());
        }
        let detail = response.body_mut().read_to_string().unwrap_or_default();
        let detail: String = detail.chars().take(200).collect();
        Err(CliError::general(format!(
            "Linear's file storage refused the upload (HTTP {status}){}",
            if detail.trim().is_empty() {
                String::new()
            } else {
                format!(": {}", detail.trim())
            }
        )))
    }

    /// Write the file at `url` to `out`, at most `max_bytes` of it. Returns how many bytes were
    /// written. A file larger than that is an error (and `out` may hold part of it).
    pub fn download(&self, url: &str, max_bytes: u64, out: &mut dyn Write) -> Result<u64> {
        let uri: Uri = url
            .parse()
            .map_err(|_| CliError::usage(format!("{url:?} is not a URL")))?;
        if !matches!(uri.scheme_str(), Some("http" | "https")) || uri.host().is_none() {
            return Err(CliError::usage(format!(
                "{url:?} is not an http(s) URL with a host"
            )));
        }
        let mut request = self.agent.get(url);
        if self.sends_credentials_to(&uri) {
            request = request.header("authorization", self.authorization()?);
        }
        let mut response = request
            .call()
            .map_err(|e| CliError::general(format!("the download failed: {}", short(&e))))?;
        let status = response.status().as_u16();
        match status {
            200..=299 => {}
            401 | 403 => {
                return Err(CliError::auth(format!(
                    "Linear refused the download (HTTP {status}): the credentials of this \
                     workspace may not read that file"
                )))
            }
            404 => return Err(CliError::general("no such file (HTTP 404)")),
            _ => {
                return Err(CliError::general(format!(
                    "the download failed (HTTP {status})"
                )))
            }
        }
        let too_big = || {
            CliError::general(format!(
                "the file is larger than the limit of {} MiB, so it was not saved",
                max_bytes / (1024 * 1024)
            ))
        };
        let declared = response
            .headers()
            .get("content-length")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.trim().parse::<u64>().ok());
        if declared.is_some_and(|n| n > max_bytes) {
            return Err(too_big());
        }
        // One byte more than the limit tells a file at the limit from one over it.
        let mut reader = response.body_mut().as_reader().take(max_bytes + 1);
        let mut buffer = [0u8; 64 * 1024];
        let mut written = 0u64;
        loop {
            let n = reader
                .read(&mut buffer)
                .map_err(|e| CliError::general(format!("the download broke off: {e}")))?;
            if n == 0 {
                break;
            }
            written += n as u64;
            if written > max_bytes {
                return Err(too_big());
            }
            out.write_all(&buffer[..n])?;
        }
        Ok(written)
    }

    /// May the workspace's credential be sent to this URL? Only to Linear's file
    /// host over https, or to the origin of the API endpoint itself.
    fn sends_credentials_to(&self, uri: &Uri) -> bool {
        if uri.scheme_str() == Some("https") && uri.host() == Some(FILE_HOST) {
            return uri.port_u16().is_none_or(|p| p == 443);
        }
        let Ok(api) = self.url.parse::<Uri>() else {
            return false;
        };
        uri.scheme() == api.scheme() && uri.authority() == api.authority()
    }
}
