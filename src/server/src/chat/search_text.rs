//! The one reader-facing text extraction over a segment list, shared by
//! `data::engine::search_text`'s `note` and `message` arms so the FTS
//! projection is never duplicated across the two doc types.

use crate::chat::{DrawnRow, Segment, TableDrawSegment};

/// Appends `s` to `out`, separated from any existing content by a single
/// space, so the accumulated text never carries doubled or leading spaces.
fn push_text(out: &mut String, s: &str) {
    if s.is_empty() {
        return;
    }
    if !out.is_empty() {
        out.push(' ');
    }
    out.push_str(s);
}

/// Decodes the small, fixed entity set ammonia emits (`&amp;` `&lt;` `&gt;`
/// `&quot;` `&#39;` and numeric `&#NNN;`/`&#xHH;`) after every `<…>` run has
/// been stripped. Safe on the stripped remainder: ammonia has already
/// escaped every literal `<` and `&` in text nodes, so no other entity forms
/// occur in sanitized output.
fn decode_entities(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'&' {
            if let Some(end) = s[i..].find(';').map(|p| i + p) {
                let entity = &s[i + 1..end];
                let decoded = match entity {
                    "amp" => Some('&'),
                    "lt" => Some('<'),
                    "gt" => Some('>'),
                    "quot" => Some('"'),
                    "#39" | "#x27" | "#X27" => Some('\''),
                    _ if entity.starts_with('#') => {
                        let numeric = &entity[1..];
                        let code = if let Some(hex) = numeric
                            .strip_prefix('x')
                            .or_else(|| numeric.strip_prefix('X'))
                        {
                            u32::from_str_radix(hex, 16).ok()
                        } else {
                            numeric.parse::<u32>().ok()
                        };
                        code.and_then(char::from_u32)
                    }
                    _ => None,
                };
                if let Some(c) = decoded {
                    out.push(c);
                    i = end + 1;
                    continue;
                }
            }
        }
        let ch = s[i..].chars().next().unwrap_or('\u{FFFD}');
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

/// Removes every `<…>` run from ammonia-sanitized HTML, then decodes the
/// entities ammonia emits for literal `&`/`<`/`>`/`"`/`'` in text nodes.
/// Safe because html5ever's `HtmlSerializer::write_escaped` escapes `<` and
/// `>` unconditionally, including inside attribute values, when ammonia
/// serializes its cleaned DOM back to a string — no code path in `clean()`'s
/// output can produce a raw `>` inside a tag, so a `<…>` run in `html` is
/// always a real element boundary, never author text that merely looks like
/// one. This is a toolchain-coupled invariant: re-verify it against
/// `ammonia`'s and `html5ever`'s serializer on any version bump of either.
fn strip_tags_and_decode(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut in_tag = false;
    for c in html.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    decode_entities(&out)
}

/// Reader-facing text over one drawn row (recursive through `nested`),
/// shared by the top-level `TableDraw` arm below.
fn push_row_text(out: &mut String, row: &DrawnRow) {
    push_text(out, &row.label);
    push_text(out, &segments_search_text(&row.content));
    for nested in &row.nested {
        push_table_draw_text(out, nested);
    }
}

/// Reader-facing text over one table draw: the table name plus, when a row
/// matched, that row's text and every nested draw's text recursively.
/// Excludes `formula`/`spec`/`raw`/`roll_id` (roll internals, not reader
/// content).
fn push_table_draw_text(out: &mut String, draw: &TableDrawSegment) {
    push_text(out, &draw.table_name);
    if let Some(row) = &draw.row {
        push_row_text(out, row);
    }
}

/// The reader-facing text a recipient sees when a segment list renders,
/// space-joined across segments. Shared by `data::engine::search_text`'s
/// `note` (`NoteEngine.body`) and `message` (`MessageEngine.content`) arms —
/// the ONE segment-list text extraction, never duplicated. Excludes every
/// structural/internal field: roll specs and raw dice logs, asset/target
/// ids, discriminant kinds, and markup tag/attribute names.
///
/// # Examples
///
/// ```
/// use shadowcat::chat::segments_search_text;
/// use shadowcat::chat::Segment;
///
/// let segments = vec![Segment::Text { text: "hello world".to_string() }];
/// assert_eq!(segments_search_text(&segments), "hello world");
/// ```
pub fn segments_search_text(segments: &[Segment]) -> String {
    let mut out = String::new();
    for segment in segments {
        match segment {
            Segment::Text { text } => push_text(&mut out, text),
            Segment::Html { sanitized_html } => {
                push_text(&mut out, &strip_tags_and_decode(sanitized_html));
            }
            Segment::RollEmbed { formula, .. } => push_text(&mut out, formula),
            Segment::RollButton { formula, label } => {
                push_text(&mut out, formula);
                if let Some(label) = label {
                    push_text(&mut out, label);
                }
            }
            Segment::LinkPreview {
                url,
                title,
                description,
                ..
            } => {
                push_text(&mut out, title);
                push_text(&mut out, description);
                push_text(&mut out, url);
            }
            Segment::OEmbed(oembed) => {
                push_text(&mut out, &oembed.provider_name);
                if let Some(title) = &oembed.title {
                    push_text(&mut out, title);
                }
                if let Some(author_name) = &oembed.author_name {
                    push_text(&mut out, author_name);
                }
            }
            Segment::DocLink { label, .. } => push_text(&mut out, label),
            Segment::Image { alt, .. } => push_text(&mut out, alt),
            Segment::TableDraw(draw) => push_table_draw_text(&mut out, draw),
        }
    }
    out
}

#[cfg(test)]
mod tests;
