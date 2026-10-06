//! Telling whether a description Linear stores is the one that was sent.
//!
//! Linear rewrites markdown as it saves it: `- item` comes back as `* item`.
//! Comparing the stored text with the text that was sent, byte for byte, would
//! make a second identical `issue update --body-file` look like a change and
//! send it again. [`same_description`] compares them in a form that ignores
//! what Linear is known to rewrite. It is deliberately modest: a rewrite it
//! does not know about only costs one redundant (and harmless) write.

/// Does Linear already store `wanted` as an issue's description?
///
/// `stored` is the description Linear returns (`None`: the issue has none).
pub fn same_description(stored: Option<&str>, wanted: &str) -> bool {
    stored.is_some_and(|s| normalize(s) == normalize(wanted))
}

/// The text with what Linear rewrites made uniform: line endings, trailing
/// whitespace, trailing blank lines and the bullet of list items (`-`, `+`
/// and `*` all become `*`).
fn normalize(text: &str) -> String {
    let lines: Vec<String> = text
        .replace("\r\n", "\n")
        .lines()
        .map(|line| {
            let line = line.trim_end();
            let rest = line.trim_start_matches([' ', '\t']);
            let indent = &line[..line.len() - rest.len()];
            match rest.strip_prefix(['-', '+', '*']) {
                Some(item) if item.starts_with(' ') => format!("{indent}*{item}"),
                _ => line.to_owned(),
            }
        })
        .collect();
    lines.join("\n").trim_end().to_owned()
}
