//! Byte-range policy for the whole-file asset responses.
//!
//! It decides which responses are ranged at all (`classify`) and what a `Range` request selects
//! from a file of known length (`decide`).
//!
//! Pure: no IO, no clock, no allocation beyond the parsed request. The serve sites and the
//! compression predicate consume the one classification, so the ranged-versus-compressed decision
//! cannot fork between them. Semantics follow RFC 9110 section 14 (range requests): a `Range`
//! header the server does not understand, or whose unit is not `bytes`, is ignored and the full
//! representation is sent; a range set with no satisfiable member is answered 416.

/// Largest slice of a media response one request is answered with: one read-ahead burst of a
/// browser media element. A response larger than the element buffers is a series of such slices,
/// each fetched when the element needs it, so no response idles unread past the egress window.
pub(super) const MAX_RANGE_SLICE: u64 = 2 * 1024 * 1024;

/// Ceiling on `range-spec`s per request. A request over it is answered in full, never partially:
/// the count bounds the number of part headers a response can carry.
pub(super) const MAX_RANGE_SPECS: usize = 16;

/// Whether a response is ranged, and how far one ranged response may extend.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum RangeClass {
    /// Compressible type on the embed route: `Range` is ignored, no `Accept-Ranges` is sent, and
    /// the compression layer may encode the body.
    Ignore,
    /// Media, raster image, PDF, archive or font served inline: ranged, never compressed, and one
    /// response carries at most `MAX_RANGE_SLICE` bytes.
    MediaSliced,
    /// Any explicit download (an attachment of a ranged type, or the retained original of any
    /// type): ranged, never compressed, and uncapped so a resume `bytes=N-` runs to the end of
    /// the file.
    DownloadFull,
}

/// The route a response is served from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Route {
    /// `http::assets::serve`: the asset embedded in the page or fetched by an element.
    Embed,
    /// `http::assets::mutate::original`: the explicit download of the retained original.
    Original,
}

/// The `Content-Disposition` disposition type a response carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Disposition {
    /// `inline`: the browser renders or plays the bytes.
    Inline,
    /// `attachment`: the browser saves the bytes.
    Attachment,
}

/// An inclusive byte range `start..=end` that lies inside the file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Slice {
    /// First byte offset of the slice.
    pub(super) start: u64,
    /// Last byte offset of the slice, inclusive; at most `len - 1`.
    pub(super) end: u64,
}

impl Slice {
    /// Number of bytes the slice selects.
    pub(super) fn len(&self) -> u64 {
        self.end - self.start + 1
    }
}

/// What a request selects from a representation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum RangeDecision {
    /// The whole representation: answered 200.
    Full,
    /// The listed slices, in ascending order and disjoint: answered 206.
    Slices(Vec<Slice>),
    /// No byte of the range set lies inside the file: answered 416.
    Unsatisfiable,
}

/// Content types, other than the `audio/`, `video/` and `image/` families, whose clients need
/// `Range` (lazy page loads, resume) or whose payload is already compressed. Compared lowercase
/// and without parameters. A ranged response cannot also be compressed (a range addresses bytes
/// of the selected representation, and a compressed slice would contradict its own
/// `Content-Range`), so membership trades compression for ranges. Names are the IANA media-type
/// registry names.
const RANGED_EXACT_TYPES: [&str; 13] = [
    "application/ogg",
    "application/pdf",
    "application/zip",
    "application/gzip",
    "application/x-gzip",
    "application/x-bzip2",
    "application/x-xz",
    "application/zstd",
    "application/x-7z-compressed",
    "application/vnd.rar",
    "application/x-rar-compressed",
    "font/woff",
    "font/woff2",
];

/// The `image/` subtype that is text and therefore compressible. It is the exclusion
/// `tower_http::compression::predicate::NotForContentType::IMAGES` encodes, which keeps the two
/// layers' image rules identical.
const COMPRESSIBLE_IMAGE_TYPE: &str = "image/svg+xml";

/// `true` when `content_type` is a media, raster image, PDF, archive or font type (see
/// `RANGED_EXACT_TYPES` and the `audio/`, `video/` and `image/` prefixes). Case-insensitive;
/// parameters after `;` are ignored, so `audio/ogg; codecs=opus` is matched by the `audio/`
/// prefix.
pub(super) fn is_media_or_archive(content_type: &str) -> bool {
    let essence = content_type
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    if essence.starts_with("audio/") || essence.starts_with("video/") {
        return true;
    }
    if essence.starts_with("image/") {
        return !essence.starts_with(COMPRESSIBLE_IMAGE_TYPE);
    }
    RANGED_EXACT_TYPES.contains(&essence.as_str())
}

/// The only place that decides whether a response is ranged and uncompressed.
///
/// `Route::Original` is an explicit save of a file: every type is `DownloadFull`, because a
/// resume matters more than the compression it costs. On `Route::Embed` a ranged type is
/// `MediaSliced` inline and `DownloadFull` as an attachment (a browser download of a zip or PDF
/// resumes to the end, not 2 MiB at a time); every other type is `Ignore`, because the attachment
/// header `serve` sends for non-raster types is a rendering-safety header, not a download intent,
/// so it must not turn an SVG or a text file into an uncompressed download.
pub(super) fn classify(content_type: &str, disposition: Disposition, route: Route) -> RangeClass {
    match route {
        Route::Original => RangeClass::DownloadFull,
        Route::Embed if !is_media_or_archive(content_type) => RangeClass::Ignore,
        Route::Embed => match disposition {
            Disposition::Inline => RangeClass::MediaSliced,
            Disposition::Attachment => RangeClass::DownloadFull,
        },
    }
}

/// One `range-spec` of a `Range` header, before it is resolved against a file length.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Spec {
    /// `a-b`: bytes `a` through `b`.
    FromTo(u64, u64),
    /// `a-`: bytes `a` through the end.
    From(u64),
    /// `-n`: the last `n` bytes.
    Suffix(u64),
}

/// Parses a non-empty run of ASCII digits, saturating at `u64::MAX`, so an overflowing position
/// still names a position beyond any file instead of being a syntax error.
fn parse_position(digits: &str) -> Option<u64> {
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let mut value: u64 = 0;
    for b in digits.bytes() {
        value = value.saturating_mul(10).saturating_add(u64::from(b - b'0'));
    }
    Some(value)
}

/// Parses one `range-spec`; `None` for any syntax error, including a first position after the
/// last.
fn parse_spec(spec: &str) -> Option<Spec> {
    let (first, last) = spec.split_once('-')?;
    match (first.is_empty(), last.is_empty()) {
        (true, true) => None,
        (true, false) => parse_position(last).map(Spec::Suffix),
        (false, true) => parse_position(first).map(Spec::From),
        (false, false) => {
            let a = parse_position(first)?;
            let b = parse_position(last)?;
            (a <= b).then_some(Spec::FromTo(a, b))
        }
    }
}

/// Parses a `Range` header into its specs. `None` when the header is to be ignored: a unit other
/// than `bytes` (the unit name is case-insensitive), no spec at all, or any syntax error.
fn parse_range(header: &str) -> Option<Vec<Spec>> {
    let (unit, set) = header.trim().split_once('=')?;
    if !unit.trim().eq_ignore_ascii_case("bytes") {
        return None;
    }
    let specs = set
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(parse_spec)
        .collect::<Option<Vec<_>>>()?;
    (!specs.is_empty()).then_some(specs)
}

/// Resolves a spec against a file of `len` bytes; `None` when it selects no byte of the file.
fn resolve(spec: Spec, len: u64) -> Option<Slice> {
    if len == 0 {
        return None;
    }
    match spec {
        Spec::FromTo(a, b) if a < len => Some(Slice {
            start: a,
            end: b.min(len - 1),
        }),
        Spec::From(a) if a < len => Some(Slice {
            start: a,
            end: len - 1,
        }),
        Spec::Suffix(n) if n > 0 => Some(Slice {
            start: len - n.min(len),
            end: len - 1,
        }),
        _ => None,
    }
}

/// `true` when `If-Range` names the current strong validator exactly. A weak validator, a
/// different tag and an HTTP-date (no `Last-Modified` is sent, so a date never validates) all
/// fail, which makes the request an ordinary full-representation GET.
fn if_range_holds(if_range: &str, etag: &str) -> bool {
    let value = if_range.trim();
    !value.starts_with("W/") && value == etag
}

/// What `range` selects from a representation of `len` bytes with validator `etag`.
///
/// `Full` when the class is `Ignore`, `range` is absent or ignorable, `if_range` is present and
/// does not validate, or the request names more than `MAX_RANGE_SPECS` ranges. Otherwise the
/// satisfiable ranges are sorted and coalesced (overlapping and adjacent ranges merge) before
/// anything is totalled, so the answer is a set of disjoint slices whose total is at most `len`.
/// A single resulting `MediaSliced` slice is shortened to `MAX_RANGE_SLICE` (a server may send
/// less than requested; the element continues from the `Content-Range` it receives); several
/// slices whose total exceeds the cap are answered in full, because a part is never shortened: a
/// truncated part would contradict its own `Content-Range`.
pub(super) fn decide(
    class: RangeClass,
    range: Option<&str>,
    if_range: Option<&str>,
    etag: &str,
    len: u64,
) -> RangeDecision {
    let Some(range) = range.filter(|_| class != RangeClass::Ignore) else {
        return RangeDecision::Full;
    };
    if if_range.is_some_and(|v| !if_range_holds(v, etag)) {
        return RangeDecision::Full;
    }
    let Some(specs) = parse_range(range) else {
        return RangeDecision::Full;
    };
    if specs.len() > MAX_RANGE_SPECS {
        return RangeDecision::Full;
    }
    let mut slices: Vec<Slice> = specs.iter().filter_map(|s| resolve(*s, len)).collect();
    if slices.is_empty() {
        return RangeDecision::Unsatisfiable;
    }
    let mut slices = coalesce(&mut slices);
    if let [slice] = slices.as_mut_slice() {
        if class == RangeClass::MediaSliced {
            slice.end = slice
                .end
                .min(slice.start.saturating_add(MAX_RANGE_SLICE - 1));
        }
    } else if class == RangeClass::MediaSliced
        && slices.iter().map(Slice::len).sum::<u64>() > MAX_RANGE_SLICE
    {
        return RangeDecision::Full;
    }
    RangeDecision::Slices(slices)
}

/// Sorts `slices` by start and merges every overlapping or adjacent pair, so the result is a set
/// of disjoint, non-adjacent parts in ascending order whose total never exceeds the file length:
/// a request cannot make the response larger than the file by repeating a range.
fn coalesce(slices: &mut [Slice]) -> Vec<Slice> {
    slices.sort_by_key(|s| s.start);
    let mut merged: Vec<Slice> = Vec::with_capacity(slices.len());
    for slice in slices.iter() {
        match merged.last_mut() {
            Some(last) if slice.start <= last.end.saturating_add(1) => {
                last.end = last.end.max(slice.end);
            }
            _ => merged.push(*slice),
        }
    }
    merged
}

/// A fresh multipart boundary: a random v4 UUID (122 random bits), so a payload containing the
/// delimiter is negligible. RFC 9110 section 14.6 allows a boundary that cannot be proven absent
/// from the payload.
pub(super) fn new_boundary() -> String {
    format!("shadowcat-{}", uuid::Uuid::new_v4().simple())
}

/// The `Content-Type` of a `multipart/byteranges` response.
pub(super) fn multipart_content_type(boundary: &str) -> String {
    format!("multipart/byteranges; boundary={boundary}")
}

/// The bytes that precede one part's payload: the delimiter line, the representation's
/// `Content-Type` and the part's `Content-Range`, then the blank line. Every part's payload is
/// followed by `PART_END`.
pub(super) fn multipart_part_head(
    boundary: &str,
    content_type: &str,
    slice: Slice,
    len: u64,
) -> String {
    format!(
        "--{boundary}\r\nContent-Type: {content_type}\r\nContent-Range: bytes {}-{}/{len}\r\n\r\n",
        slice.start, slice.end
    )
}

/// Terminates a part's payload.
pub(super) const PART_END: &str = "\r\n";

/// The close delimiter that ends a `multipart/byteranges` body.
pub(super) fn multipart_close(boundary: &str) -> String {
    format!("--{boundary}--\r\n")
}

/// The exact byte length of a `multipart/byteranges` body: every part head, payload and
/// terminator, then the close delimiter. An exact length lets the response carry
/// `Content-Length` instead of chunked framing.
pub(super) fn multipart_len(boundary: &str, content_type: &str, slices: &[Slice], len: u64) -> u64 {
    let parts: u64 = slices
        .iter()
        .map(|slice| {
            multipart_part_head(boundary, content_type, *slice, len).len() as u64
                + slice.len()
                + PART_END.len() as u64
        })
        .sum();
    parts + multipart_close(boundary).len() as u64
}

#[cfg(test)]
mod tests;
