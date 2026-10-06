//! Request building, response parsing and pagination.
//!
//! Nothing here performs I/O. A caller builds a [`Request`], sends it however
//! it likes, then hands the status/headers and body back to [`parse_response`].

use crate::error::{Error, Result};
use crate::types::{PageInfo, PageVars};
use chrono::{DateTime, Utc};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

/// A GraphQL request body, ready to be sent as JSON.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Request {
    pub query: String,
    #[serde(skip_serializing_if = "serde_json::Value::is_null")]
    pub variables: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operation_name: Option<String>,
}

impl Request {
    /// The JSON text to send as the request body.
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).expect("a request always serializes")
    }
}

/// Build a request from a cynic operation.
pub fn build_request<Q, V: Serialize>(op: &cynic::Operation<Q, V>) -> Request {
    Request {
        query: op.query.clone(),
        variables: serde_json::to_value(&op.variables).expect("variables always serialize"),
        operation_name: op.operation_name.as_ref().map(|n| n.to_string()),
    }
}

/// What the parser needs to know about the HTTP response besides its body.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResponseMeta {
    pub status: u16,
    /// `Retry-After`, in seconds.
    pub retry_after_secs: Option<u64>,
    /// `X-RateLimit-Requests-Reset` (or the complexity equivalent): epoch milliseconds.
    pub rate_limit_reset_ms: Option<i64>,
}

#[derive(Deserialize)]
struct Envelope {
    data: Option<serde_json::Value>,
    errors: Option<Vec<GraphqlError>>,
}

#[derive(Deserialize)]
struct GraphqlError {
    message: String,
    #[serde(default)]
    extensions: Option<Extensions>,
}

#[derive(Deserialize, Default)]
struct Extensions {
    #[serde(rename = "type")]
    kind: Option<String>,
    code: Option<String>,
    #[serde(rename = "userPresentableMessage")]
    presentable: Option<String>,
}

/// Interpret a response.
///
/// Partial data alongside errors is treated as a failure: a command that
/// silently works from half a response is how "quietly empty" bugs happen.
/// `now` is only used to turn a rate-limit reset timestamp into a wait.
pub fn parse_response<T: DeserializeOwned>(
    meta: &ResponseMeta,
    body: &str,
    now: DateTime<Utc>,
) -> Result<T> {
    let envelope: Option<Envelope> = serde_json::from_str(body).ok();

    if let Some(env) = &envelope {
        if let Some(errors) = env.errors.as_ref().filter(|e| !e.is_empty()) {
            return Err(classify(meta, errors, now));
        }
    }

    if meta.status == 401 || meta.status == 403 {
        return Err(Error::Auth(format!("HTTP {}", meta.status)));
    }
    if meta.status == 429 {
        return Err(Error::RateLimited {
            retry_after_secs: retry_after(meta, now),
        });
    }
    if !(200..300).contains(&meta.status) {
        return Err(Error::Http {
            status: meta.status,
            body: truncate(body, 300),
        });
    }

    let env = envelope.ok_or_else(|| Error::Decode("the body is not a GraphQL response".into()))?;
    let data = env.data.ok_or(Error::Empty)?;
    serde_json::from_value(data).map_err(|e| Error::Decode(e.to_string()))
}

fn classify(meta: &ResponseMeta, errors: &[GraphqlError], now: DateTime<Utc>) -> Error {
    let first = &errors[0];
    let ext = first.extensions.as_ref();
    let code = ext.and_then(|e| e.code.clone());
    let kind = ext.and_then(|e| e.kind.as_deref()).unwrap_or("");

    let message = errors
        .iter()
        .map(|e| {
            let hint = e.extensions.as_ref().and_then(|x| x.presentable.as_deref());
            match hint {
                Some(h) if !h.is_empty() && h != e.message => format!("{}: {h}", e.message),
                _ => e.message.clone(),
            }
        })
        .collect::<Vec<_>>()
        .join("; ");

    match (code.as_deref(), kind) {
        (Some("RATELIMITED"), _) | (_, "ratelimited") => Error::RateLimited {
            retry_after_secs: retry_after(meta, now),
        },
        (Some("AUTHENTICATION_ERROR" | "FORBIDDEN"), _)
        | (_, "authentication error" | "forbidden") => Error::Auth(message),
        _ => Error::Api { message, code },
    }
}

fn retry_after(meta: &ResponseMeta, now: DateTime<Utc>) -> Option<u64> {
    if let Some(s) = meta.retry_after_secs {
        return Some(s);
    }
    let reset = meta.rate_limit_reset_ms?;
    let wait_ms = reset - now.timestamp_millis();
    Some(if wait_ms <= 0 {
        0
    } else {
        (wait_ms as u64).div_ceil(1000)
    })
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_owned()
    } else {
        let head: String = s.chars().take(max).collect();
        format!("{head}...")
    }
}

// ------------------------------------------------------------------ pagination

/// A paginated connection: a page of items plus the information to continue.
pub trait Page {
    type Item;
    fn page_info(&self) -> &PageInfo;
    fn into_items(self) -> Vec<Self::Item>;
}

/// Default ceiling on pages per listing; a guard against runaway loops.
pub const DEFAULT_MAX_PAGES: usize = 200;

/// Drives cursor pagination without doing any I/O.
///
/// ```text
/// while let Some(vars) = pager.next_vars() {
///     let page = fetch(vars)?;          // caller's I/O
///     items.extend(pager.accept(page)?);
/// }
/// ```
#[derive(Debug, Clone)]
pub struct Pager {
    page_size: i32,
    max_pages: usize,
    after: Option<String>,
    pages: usize,
    done: bool,
}

impl Pager {
    pub fn new(page_size: i32) -> Self {
        Self {
            page_size,
            max_pages: DEFAULT_MAX_PAGES,
            after: None,
            pages: 0,
            done: false,
        }
    }

    pub fn with_max_pages(mut self, max_pages: usize) -> Self {
        self.max_pages = max_pages;
        self
    }

    /// The variables for the next request, or `None` once the last page was accepted.
    pub fn next_vars(&self) -> Option<PageVars> {
        if self.done {
            return None;
        }
        Some(PageVars {
            first: self.page_size,
            after: self.after.clone(),
        })
    }

    /// Take a fetched page: returns its items and records where to continue.
    pub fn accept<P: Page>(&mut self, page: P) -> Result<Vec<P::Item>> {
        self.pages += 1;
        let info = page.page_info().clone();
        if info.has_next_page {
            let cursor = info.end_cursor.ok_or_else(|| {
                Error::Pagination("the page has more results but no end cursor".into())
            })?;
            if self.after.as_deref() == Some(cursor.as_str()) {
                return Err(Error::Pagination(format!(
                    "the cursor did not advance (stuck at {cursor})"
                )));
            }
            if self.pages >= self.max_pages {
                return Err(Error::Pagination(format!(
                    "stopped after {} pages; the listing is larger than the limit",
                    self.max_pages
                )));
            }
            self.after = Some(cursor);
        } else {
            self.done = true;
        }
        Ok(page.into_items())
    }

    /// Fetch every page with `fetch` (which performs the I/O) and collect the items.
    pub fn collect_all<P, F>(mut self, mut fetch: F) -> Result<Vec<P::Item>>
    where
        P: Page,
        F: FnMut(PageVars) -> Result<P>,
    {
        let mut items = Vec::new();
        while let Some(vars) = self.next_vars() {
            items.extend(self.accept(fetch(vars)?)?);
        }
        Ok(items)
    }
}
