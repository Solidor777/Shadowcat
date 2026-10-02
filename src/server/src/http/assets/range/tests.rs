//! Tables over `classify`, `is_media_or_archive` and `decide`.

use super::*;
use tower_http::compression::predicate::{NotForContentType, Predicate};

/// A strong validator in the shape the serve sites send.
const ETAG: &str = "\"11111111-1111-1111-1111-111111111111-3\"";

/// 1 MiB, small enough that no `MediaSliced` cap applies.
const SMALL: u64 = 1024 * 1024;

/// 10 MiB, larger than `MAX_RANGE_SLICE`.
const LARGE: u64 = 10 * 1024 * 1024;

fn s(start: u64, end: u64) -> Slice {
    Slice { start, end }
}

fn one(start: u64, end: u64) -> RangeDecision {
    RangeDecision::Slices(vec![s(start, end)])
}

fn media(range: &str, len: u64) -> RangeDecision {
    decide(RangeClass::MediaSliced, Some(range), None, ETAG, len)
}

fn download(range: &str, len: u64) -> RangeDecision {
    decide(RangeClass::DownloadFull, Some(range), None, ETAG, len)
}

#[test]
fn classification_table() {
    use Disposition::{Attachment, Inline};
    use RangeClass::{DownloadFull, Ignore, MediaSliced};
    use Route::{Embed, Original};
    let rows: &[(&str, Disposition, Route, RangeClass)] = &[
        ("audio/ogg", Inline, Embed, MediaSliced),
        ("audio/ogg; codecs=opus", Inline, Embed, MediaSliced),
        ("audio/webm; codecs=opus", Inline, Embed, MediaSliced),
        ("AUDIO/WAV", Inline, Embed, MediaSliced),
        ("audio/mpeg", Attachment, Embed, DownloadFull),
        ("video/mp4", Inline, Embed, MediaSliced),
        ("image/webp", Inline, Embed, MediaSliced),
        ("image/png", Inline, Embed, MediaSliced),
        ("image/svg+xml", Attachment, Embed, Ignore),
        ("image/svg+xml; charset=utf-8", Attachment, Embed, Ignore),
        ("application/pdf", Inline, Embed, MediaSliced),
        ("application/pdf", Attachment, Embed, DownloadFull),
        ("application/zip", Attachment, Embed, DownloadFull),
        ("application/ogg", Inline, Embed, MediaSliced),
        ("font/woff2", Inline, Embed, MediaSliced),
        ("font/ttf", Attachment, Embed, Ignore),
        ("text/plain", Attachment, Embed, Ignore),
        ("application/json", Attachment, Embed, Ignore),
        ("application/octet-stream", Attachment, Embed, Ignore),
        (
            "application/octet-stream",
            Attachment,
            Original,
            DownloadFull,
        ),
        ("text/plain", Attachment, Original, DownloadFull),
        ("image/svg+xml", Attachment, Original, DownloadFull),
        ("audio/ogg", Inline, Original, DownloadFull),
    ];
    for (content_type, disposition, route, expected) in rows {
        assert_eq!(
            classify(content_type, *disposition, *route),
            *expected,
            "{content_type} {disposition:?} {route:?}"
        );
    }
}

/// Every `image/*` type is ranged exactly when tower-http's own image rule leaves it
/// uncompressed, so the range classification and the compression layer cannot disagree on images.
#[test]
fn image_rule_matches_the_compression_layers_image_rule() {
    for content_type in [
        "image/png",
        "image/jpeg",
        "image/gif",
        "image/webp",
        "image/bmp",
        "image/tiff",
        "image/avif",
        "image/svg+xml",
        "image/svg+xml; charset=utf-8",
        "image/vnd.microsoft.icon",
    ] {
        let response = axum::http::Response::builder()
            .header(axum::http::header::CONTENT_TYPE, content_type)
            .body(axum::body::Body::empty())
            .unwrap();
        let compressible = NotForContentType::IMAGES.should_compress(&response);
        assert_eq!(
            is_media_or_archive(content_type),
            !compressible,
            "{content_type}: ranged iff tower-http does not compress it"
        );
    }
}

#[test]
fn ignore_class_never_ranges() {
    for range in ["bytes=0-0", "bytes=0-", "bytes=-1", "bytes=999999999-"] {
        assert_eq!(
            decide(RangeClass::Ignore, Some(range), None, ETAG, SMALL),
            RangeDecision::Full,
            "{range}"
        );
    }
}

#[test]
fn single_range_forms() {
    assert_eq!(media("bytes=0-0", 100), one(0, 0));
    assert_eq!(media("bytes=10-19", 100), one(10, 19));
    assert_eq!(media("bytes=0-", 100), one(0, 99));
    assert_eq!(media("bytes=95-", 100), one(95, 99));
    assert_eq!(media("bytes=-1", 100), one(99, 99));
    assert_eq!(media("bytes=-10", 100), one(90, 99));
    // A suffix longer than the file selects the whole file.
    assert_eq!(media("bytes=-1000", 100), one(0, 99));
    // The last position clamps to the final byte.
    assert_eq!(media("bytes=90-5000", 100), one(90, 99));
    assert_eq!(media("bytes=0-99", 100), one(0, 99));
}

#[test]
fn unsatisfiable_boundaries() {
    // a == len is the first position past the end.
    assert_eq!(media("bytes=100-", 100), RangeDecision::Unsatisfiable);
    assert_eq!(media("bytes=100-200", 100), RangeDecision::Unsatisfiable);
    assert_eq!(media("bytes=101-", 100), RangeDecision::Unsatisfiable);
    assert_eq!(media("bytes=99-", 100), one(99, 99));
    assert_eq!(media("bytes=-0", 100), RangeDecision::Unsatisfiable);
    // An empty representation has no satisfiable range.
    assert_eq!(media("bytes=0-", 0), RangeDecision::Unsatisfiable);
    assert_eq!(media("bytes=-1", 0), RangeDecision::Unsatisfiable);
    // A one-byte representation.
    assert_eq!(media("bytes=0-", 1), one(0, 0));
    assert_eq!(media("bytes=1-", 1), RangeDecision::Unsatisfiable);
    assert_eq!(media("bytes=-5", 1), one(0, 0));
    // An overflowing position saturates to a position past any file.
    assert_eq!(
        media("bytes=99999999999999999999999-", 100),
        RangeDecision::Unsatisfiable
    );
    assert_eq!(media("bytes=0-99999999999999999999999", 100), one(0, 99));
    assert_eq!(media("bytes=-99999999999999999999999", 100), one(0, 99));
    // 2^64 would wrap to 0 under wrapping arithmetic; saturation keeps it past the file.
    assert_eq!(
        media("bytes=18446744073709551616-", 100),
        RangeDecision::Unsatisfiable
    );
    assert_eq!(media("bytes=0-18446744073709551616", 100), one(0, 99));
    assert_eq!(
        media("bytes=18446744073709551615-", 100),
        RangeDecision::Unsatisfiable
    );
}

#[test]
fn ignorable_headers_answer_in_full() {
    for range in [
        "items=0-9",
        "bytes",
        "bytes=",
        "bytes=,",
        "bytes=-",
        "bytes=a-b",
        "bytes=1-2-3",
        "bytes=+1-2",
        "bytes=1 -2",
        "bytes= 1-2 3",
        "bytes=0x1-2",
        // The first position after the last is a syntax error, not an unsatisfiable range.
        "bytes=9-5",
        "",
    ] {
        assert_eq!(media(range, 100), RangeDecision::Full, "{range:?}");
    }
    // The unit name and the whitespace around the set are not significant.
    assert_eq!(media("Bytes=0-9", 100), one(0, 9));
    assert_eq!(media("  bytes=0-9  ", 100), one(0, 9));
    assert_eq!(media("bytes= 0-9", 100), one(0, 9));
}

#[test]
fn media_slices_are_capped_and_downloads_are_not() {
    assert_eq!(media("bytes=0-", LARGE), one(0, MAX_RANGE_SLICE - 1));
    assert_eq!(
        media("bytes=5-9999999999", LARGE),
        one(5, 5 + MAX_RANGE_SLICE - 1)
    );
    // A large suffix starts at len - n and is shortened from the back of the slice.
    assert_eq!(
        media("bytes=-9437184", LARGE),
        one(LARGE - 9_437_184, LARGE - 9_437_184 + MAX_RANGE_SLICE - 1)
    );
    // A range exactly at the cap is not shortened; one byte more is.
    assert_eq!(media("bytes=0-2097151", LARGE), one(0, MAX_RANGE_SLICE - 1));
    assert_eq!(media("bytes=0-2097152", LARGE), one(0, MAX_RANGE_SLICE - 1));
    // A download resumes to the end of the file.
    assert_eq!(download("bytes=0-", LARGE), one(0, LARGE - 1));
    assert_eq!(download("bytes=5000000-", LARGE), one(5_000_000, LARGE - 1));
    assert_eq!(download("bytes=-100", LARGE), one(LARGE - 100, LARGE - 1));
}

#[test]
fn if_range_gates_the_range() {
    let decide_with = |if_range: Option<&str>| {
        decide(
            RangeClass::MediaSliced,
            Some("bytes=10-19"),
            if_range,
            ETAG,
            100,
        )
    };
    assert_eq!(decide_with(None), one(10, 19));
    assert_eq!(decide_with(Some(ETAG)), one(10, 19));
    assert_eq!(decide_with(Some(&format!("  {ETAG} "))), one(10, 19));
    // A different tag, a weak validator and a date never validate.
    assert_eq!(
        decide_with(Some("\"11111111-1111-1111-1111-111111111111-2\"")),
        RangeDecision::Full
    );
    assert_eq!(decide_with(Some(&format!("W/{ETAG}"))), RangeDecision::Full);
    assert_eq!(
        decide_with(Some("Wed, 21 Oct 2015 07:28:00 GMT")),
        RangeDecision::Full
    );
    // `If-Range` without `Range` is ignored.
    assert_eq!(
        decide(RangeClass::MediaSliced, None, Some(ETAG), ETAG, 100),
        RangeDecision::Full
    );
    assert_eq!(
        decide(RangeClass::MediaSliced, None, Some("junk"), ETAG, 100),
        RangeDecision::Full
    );
    // A failed `If-Range` turns an unsatisfiable range into the full representation too.
    assert_eq!(
        decide(
            RangeClass::MediaSliced,
            Some("bytes=500-"),
            Some("\"other\""),
            ETAG,
            100
        ),
        RangeDecision::Full
    );
}

#[test]
fn slice_length_counts_both_ends() {
    assert_eq!(s(0, 0).len(), 1);
    assert_eq!(s(10, 19).len(), 10);
}

fn parts(decision: RangeDecision) -> Vec<Slice> {
    match decision {
        RangeDecision::Slices(parts) => parts,
        other => panic!("expected slices, got {other:?}"),
    }
}

#[test]
fn coalescing_table() {
    // Unordered input is sorted.
    assert_eq!(
        parts(download("bytes=50-59,0-9,20-29", 100)),
        vec![s(0, 9), s(20, 29), s(50, 59)]
    );
    // Duplicates and contained ranges merge.
    assert_eq!(parts(download("bytes=5-9,5-9", 100)), vec![s(5, 9)]);
    assert_eq!(parts(download("bytes=0-50,10-20", 100)), vec![s(0, 50)]);
    // Overlapping ranges merge.
    assert_eq!(parts(download("bytes=0-10,5-20", 100)), vec![s(0, 20)]);
    // Adjacent ranges merge; a gap of one byte does not.
    assert_eq!(parts(download("bytes=0-9,10-19", 100)), vec![s(0, 19)]);
    assert_eq!(
        parts(download("bytes=0-9,11-19", 100)),
        vec![s(0, 9), s(11, 19)]
    );
    // A chain of adjacent ranges collapses to one part.
    assert_eq!(
        parts(download("bytes=20-29,0-9,10-19", 100)),
        vec![s(0, 29)]
    );
    // Open-ended and suffix specs take part in the merge.
    assert_eq!(parts(download("bytes=90-,-5", 100)), vec![s(90, 99)]);
    assert_eq!(
        parts(download("bytes=0-4,-5", 100)),
        vec![s(0, 4), s(95, 99)]
    );
    // Whitespace around the commas and empty list elements are not significant.
    assert_eq!(
        parts(download("bytes=0-4 , 10-14,,20-24", 100)),
        vec![s(0, 4), s(10, 14), s(20, 24)]
    );
}

#[test]
fn unsatisfiable_members_are_dropped() {
    assert_eq!(
        download("bytes=200-300,400-", 100),
        RangeDecision::Unsatisfiable
    );
    assert_eq!(download("bytes=-0,100-", 100), RangeDecision::Unsatisfiable);
    // One satisfiable member among unsatisfiable ones is a single ordinary part.
    assert_eq!(download("bytes=200-300,5-9,-0", 100), one(5, 9));
    assert_eq!(media("bytes=200-300,5-9,-0", 100), one(5, 9));
    // A syntax error anywhere ignores the whole header.
    assert_eq!(download("bytes=5-9,x-y", 100), RangeDecision::Full);
    assert_eq!(download("bytes=5-9,9-5", 100), RangeDecision::Full);
}

/// Sixteen `bytes=0-` specs are one part: the response is the file once, never sixteen times.
#[test]
fn repeated_open_ranges_cannot_amplify() {
    let header = format!("bytes={}", vec!["0-"; MAX_RANGE_SPECS].join(","));
    assert_eq!(download(&header, LARGE), one(0, LARGE - 1));
    // Against a media asset the same request coalesces to one slice at the cap.
    assert_eq!(media(&header, LARGE), one(0, MAX_RANGE_SLICE - 1));
    // A set of overlapping ranges never totals more than the file.
    let overlapping = format!(
        "bytes={}",
        (0..MAX_RANGE_SPECS)
            .map(|i| format!("{}-", i * 1000))
            .collect::<Vec<_>>()
            .join(",")
    );
    let total: u64 = parts(download(&overlapping, LARGE))
        .iter()
        .map(Slice::len)
        .sum();
    assert_eq!(total, LARGE);
}

#[test]
fn spec_count_cap_is_sixteen() {
    let specs = |n: usize| {
        format!(
            "bytes={}",
            (0..n)
                .map(|i| format!("{}-{}", i * 10, i * 10 + 4))
                .collect::<Vec<_>>()
                .join(",")
        )
    };
    assert_eq!(parts(download(&specs(16), 1000)).len(), 16);
    assert_eq!(download(&specs(17), 1000), RangeDecision::Full);
    // The count is of specs, not of coalesced parts: 17 identical specs are still over the cap.
    let identical = format!("bytes={}", vec!["0-4"; MAX_RANGE_SPECS + 1].join(","));
    assert_eq!(download(&identical, 1000), RangeDecision::Full);
}

#[test]
fn media_multi_part_total_is_capped_without_truncating_a_part() {
    // Two parts whose coalesced total is exactly the cap are served with every part intact.
    let half = MAX_RANGE_SLICE / 2;
    let at_cap = format!("bytes=0-{},{}-{}", half - 1, 4 * half, 5 * half - 1);
    assert_eq!(
        parts(media(&at_cap, LARGE)),
        vec![s(0, half - 1), s(4 * half, 5 * half - 1)]
    );
    // One byte more answers in full; no part is shortened.
    let over = format!("bytes=0-{},{}-{}", half - 1, 4 * half, 5 * half);
    assert_eq!(media(&over, LARGE), RangeDecision::Full);
    // The same set is a multipart answer for a download class.
    assert_eq!(
        parts(download(&over, LARGE)),
        vec![s(0, half - 1), s(4 * half, 5 * half)]
    );
    // Sixteen disjoint small parts are a multipart response, seventeen are a full response.
    let disjoint = |n: usize| {
        format!(
            "bytes={}",
            (0..n)
                .map(|i| format!("{}-{}", i * 100, i * 100 + 9))
                .collect::<Vec<_>>()
                .join(",")
        )
    };
    assert_eq!(parts(media(&disjoint(16), LARGE)).len(), 16);
    assert_eq!(media(&disjoint(17), LARGE), RangeDecision::Full);
}

#[test]
fn a_coalesced_single_part_is_shortened_like_a_single_spec() {
    assert_eq!(media("bytes=0-,0-", LARGE), one(0, MAX_RANGE_SLICE - 1));
    assert_eq!(
        media("bytes=0-100,50-9999999", LARGE),
        one(0, MAX_RANGE_SLICE - 1)
    );
    // Equal to the single-spec answer for the same coalesced range.
    assert_eq!(media("bytes=0-,0-", LARGE), media("bytes=0-", LARGE));
}

/// Parses a `multipart/byteranges` body back into its parts, checking the framing byte for byte.
fn parse_multipart(body: &[u8], boundary: &str, content_type: &str) -> Vec<(Slice, Vec<u8>)> {
    let mut rest = body;
    let mut found = Vec::new();
    loop {
        let delimiter = format!("--{boundary}");
        assert!(rest.starts_with(delimiter.as_bytes()), "delimiter expected");
        rest = &rest[delimiter.len()..];
        if rest.starts_with(b"--\r\n") {
            assert_eq!(rest, b"--\r\n", "nothing follows the close delimiter");
            return found;
        }
        assert!(rest.starts_with(b"\r\n"));
        rest = &rest[2..];
        let end = rest
            .windows(4)
            .position(|w| w == b"\r\n\r\n")
            .expect("blank line ends the part headers");
        let headers = std::str::from_utf8(&rest[..end]).unwrap();
        rest = &rest[end + 4..];
        let mut lines = headers.split("\r\n");
        assert_eq!(
            lines.next(),
            Some(format!("Content-Type: {content_type}").as_str())
        );
        let range = lines
            .next()
            .and_then(|l| l.strip_prefix("Content-Range: bytes "))
            .expect("Content-Range header");
        assert_eq!(lines.next(), None);
        let (span, _total) = range.split_once('/').unwrap();
        let (start, end) = span.split_once('-').unwrap();
        let slice = s(start.parse().unwrap(), end.parse().unwrap());
        let payload = rest[..slice.len() as usize].to_vec();
        rest = &rest[slice.len() as usize..];
        assert!(rest.starts_with(PART_END.as_bytes()));
        rest = &rest[PART_END.len()..];
        found.push((slice, payload));
    }
}

#[test]
fn multipart_framing_has_an_exact_length_and_parses_back() {
    let file: Vec<u8> = (0..1000u32).map(|i| (i % 251) as u8).collect();
    let boundary = new_boundary();
    assert!(boundary.starts_with("shadowcat-"));
    assert_eq!(
        boundary.len(),
        "shadowcat-".len() + 32,
        "a v4 UUID in simple form"
    );
    assert_ne!(boundary, new_boundary(), "fresh per response");
    let content_type = "audio/ogg";
    let slices = vec![s(0, 9), s(100, 199), s(990, 999)];
    let mut body = Vec::new();
    for slice in &slices {
        body.extend(multipart_part_head(&boundary, content_type, *slice, 1000).bytes());
        body.extend(&file[slice.start as usize..=slice.end as usize]);
        body.extend(PART_END.bytes());
    }
    body.extend(multipart_close(&boundary).bytes());
    assert_eq!(
        multipart_len(&boundary, content_type, &slices, 1000),
        body.len() as u64
    );
    assert_eq!(
        multipart_content_type(&boundary),
        format!("multipart/byteranges; boundary={boundary}")
    );
    let found = parse_multipart(&body, &boundary, content_type);
    assert_eq!(found.len(), slices.len());
    for ((slice, payload), expected) in found.iter().zip(&slices) {
        assert_eq!(slice, expected);
        assert_eq!(payload, &file[slice.start as usize..=slice.end as usize]);
    }
}
