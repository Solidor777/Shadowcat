//! The one decision between "ranged and uncompressed" and "compressed", read through the real
//! router with `Accept-Encoding: gzip` on every request.
//!
//! `http::assets::range::classify` decides the class and `serve_file` writes `Accept-Ranges`
//! exactly when the class is not `Ignore`; the compression layer's predicate refuses any response
//! that carries `Accept-Ranges`. So for every (type, disposition, route) row the response has
//! `Accept-Ranges` if and only if it is not `Content-Encoding`-compressed, and the table here is
//! the contract a later narrowing of the type list must not silently change.

use shadowcat::data::asset::AssetMeta;
use shadowcat_test_support as common;
use shadowcat_test_support::Harness;
use uuid::Uuid;

/// 2 MiB: the most one inline media response carries.
const SLICE: usize = 2 * 1024 * 1024;

/// A seeded file larger than one slice, compressible so gzip is worth applying where it is.
const SIZE: usize = 3 * 1024 * 1024 + 17;

/// Bytes whose value depends on the offset, so a wrongly placed slice is visible. Highly
/// repetitive, so every compressible type shrinks under gzip.
fn pattern(len: usize) -> Vec<u8> {
    (0..len).map(|i| ((i * 7 + i / 251) % 256) as u8).collect()
}

/// What the table says a row is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Expect {
    /// Ranged, uncompressed, one response carries at most one slice.
    Sliced,
    /// Ranged, uncompressed, a response may run to the end of the file.
    Whole,
    /// Compressed, no `Accept-Ranges`, `Range` ignored.
    Compressed,
}

/// One row: a stored type, where it is read from and what it must be.
struct Row {
    /// Names the row in assertion messages.
    label: &'static str,
    /// The asset's stored `Content-Type`.
    stored: &'static str,
    /// Suffix of the file the row's URL reads (`""` is the canonical file).
    suffix: &'static str,
    /// The request's query string.
    query: &'static str,
    /// The class the row must have.
    expect: Expect,
}

/// The `Content-Type` a request to `/api/assets/{id}` answers with for a row whose query selects
/// no variant.
const SERVE_ROWS: &[Row] = &[
    Row {
        label: "audio/ogg canonical (attachment)",
        stored: "audio/ogg",
        suffix: "",
        query: "",
        expect: Expect::Whole,
    },
    Row {
        label: "audio/wav canonical: raw PCM trades its compression for seeking",
        stored: "audio/wav",
        suffix: "",
        query: "",
        expect: Expect::Whole,
    },
    Row {
        label: "opus derivative (audio/ogg; codecs=opus, inline)",
        stored: "audio/wav",
        suffix: ".opus.ogg",
        query: "?variant=opus",
        expect: Expect::Sliced,
    },
    Row {
        label: "opus-webm derivative (audio/webm; codecs=opus, inline)",
        stored: "audio/wav",
        suffix: ".opus.webm",
        query: "?variant=opus-webm",
        expect: Expect::Sliced,
    },
    Row {
        label: "image/webp canonical (inline)",
        stored: "image/webp",
        suffix: "",
        query: "",
        expect: Expect::Sliced,
    },
    Row {
        label: "image/png canonical (inline)",
        stored: "image/png",
        suffix: "",
        query: "",
        expect: Expect::Sliced,
    },
    Row {
        label: "application/zip canonical (attachment)",
        stored: "application/zip",
        suffix: "",
        query: "",
        expect: Expect::Whole,
    },
    Row {
        label: "application/pdf canonical (attachment)",
        stored: "application/pdf",
        suffix: "",
        query: "",
        expect: Expect::Whole,
    },
    Row {
        label: "font/woff2 canonical (attachment)",
        stored: "font/woff2",
        suffix: "",
        query: "",
        expect: Expect::Whole,
    },
    Row {
        label: "image/svg+xml canonical (attachment, but compressible)",
        stored: "image/svg+xml",
        suffix: "",
        query: "",
        expect: Expect::Compressed,
    },
    Row {
        label: "text/plain canonical",
        stored: "text/plain",
        suffix: "",
        query: "",
        expect: Expect::Compressed,
    },
    Row {
        label: "application/octet-stream canonical",
        stored: "application/octet-stream",
        suffix: "",
        query: "",
        expect: Expect::Compressed,
    },
    Row {
        label: "font/ttf canonical",
        stored: "font/ttf",
        suffix: "",
        query: "",
        expect: Expect::Compressed,
    },
];

/// `GET url` with gzip accepted and `range`, if any.
async fn fetch(h: &Harness, url: &str, range: Option<&str>) -> reqwest::Response {
    let mut request = h.client.get(url).header("accept-encoding", "gzip");
    if let Some(range) = range {
        request = request.header("range", range);
    }
    request.send().await.unwrap()
}

fn header(res: &reqwest::Response, name: &str) -> Option<String> {
    res.headers()
        .get(name)
        .map(|v| v.to_str().unwrap().to_string())
}

/// Asserts `url` behaves as `expect` says, with `label` in every message.
async fn assert_class(h: &Harness, url: &str, label: &str, expect: Expect, bytes: &[u8]) {
    let res = fetch(h, url, None).await;
    assert_eq!(res.status(), 200, "{label}: plain GET");
    match expect {
        Expect::Sliced | Expect::Whole => {
            assert_eq!(
                header(&res, "accept-ranges").as_deref(),
                Some("bytes"),
                "{label}"
            );
            assert!(
                header(&res, "content-encoding").is_none(),
                "{label}: a ranged type must not be compressed"
            );
            assert_eq!(
                header(&res, "content-length"),
                Some(bytes.len().to_string()),
                "{label}"
            );
            assert_eq!(res.bytes().await.unwrap().as_ref(), bytes, "{label}");

            let res = fetch(h, url, Some("bytes=10-19")).await;
            assert_eq!(res.status(), 206, "{label}: bytes=10-19");
            assert!(header(&res, "content-encoding").is_none(), "{label}");
            assert_eq!(
                header(&res, "content-range"),
                Some(format!("bytes 10-19/{}", bytes.len())),
                "{label}"
            );
            assert_eq!(
                res.bytes().await.unwrap().as_ref(),
                &bytes[10..20],
                "{label}"
            );

            let res = fetch(h, url, Some("bytes=0-")).await;
            assert_eq!(res.status(), 206, "{label}: bytes=0-");
            let got = res.bytes().await.unwrap();
            let want = if expect == Expect::Sliced {
                SLICE
            } else {
                bytes.len()
            };
            assert_eq!(got.len(), want, "{label}: open-ended range length");
            assert_eq!(got.as_ref(), &bytes[..want], "{label}");
        }
        Expect::Compressed => {
            assert!(
                header(&res, "accept-ranges").is_none(),
                "{label}: a compressed type advertises no ranges"
            );
            assert_eq!(
                header(&res, "content-encoding").as_deref(),
                Some("gzip"),
                "{label}"
            );
            let body = res.bytes().await.unwrap();
            assert_eq!(&body[..2], &[0x1f, 0x8b], "{label}: gzip magic");
            assert!(body.len() < bytes.len(), "{label}: gzip must shrink it");

            // `Range` is ignored: the answer is the whole compressed 200.
            let res = fetch(h, url, Some("bytes=10-19")).await;
            assert_eq!(res.status(), 200, "{label}: Range is ignored");
            assert!(header(&res, "content-range").is_none(), "{label}");
            assert_eq!(
                header(&res, "content-encoding").as_deref(),
                Some("gzip"),
                "{label}"
            );
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn every_type_class_disposition_and_route_row_is_ranged_or_compressed_as_the_table_says() {
    let h = common::spawn().await;
    let bytes = pattern(SIZE);
    for row in SERVE_ROWS {
        let id = h
            .seed_asset(
                row.stored,
                AssetMeta::unprocessed(row.stored, 0),
                &[(row.suffix, &bytes)],
            )
            .await;
        // A canonical row keeps the canonical file present even when the row reads a derivative.
        if !row.suffix.is_empty() {
            std::fs::write(
                h.assets_dir.join(h.world.to_string()).join(id.to_string()),
                b"x",
            )
            .unwrap();
        }
        let url = format!("http://{}/api/assets/{id}{}", h.addr, row.query);
        assert_class(&h, &url, row.label, row.expect, &bytes).await;
    }
}

/// The retained-original download is the explicit-download route: every type, compressible ones
/// included, is ranged and uncompressed.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_retained_original_is_never_compressed_whatever_its_type() {
    let h = common::spawn().await;
    let bytes = pattern(SIZE);
    for original_type in [
        "text/plain",
        "image/svg+xml",
        "application/octet-stream",
        "application/json",
    ] {
        let mut meta = AssetMeta::unprocessed("image/webp", 1);
        meta.original_retained = true;
        meta.original_content_type = original_type.into();
        let id: Uuid = h
            .seed_asset("image/webp", meta, &[("", b"x"), (".orig", &bytes)])
            .await;
        let url = format!("http://{}/api/assets/{id}/original", h.addr);
        assert_class(&h, &url, original_type, Expect::Whole, &bytes).await;
    }
}

/// A 416 and a 304 carry no encoded body, and a HEAD of a ranged type advertises ranges and no
/// encoding, so a proxy or download manager probing with HEAD sees the same policy.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn head_416_and_304_of_a_ranged_type_carry_no_content_encoding() {
    let h = common::spawn().await;
    let bytes = pattern(SIZE);
    let id = h
        .seed_asset(
            "audio/wav",
            AssetMeta::unprocessed("audio/wav", 0),
            &[(".opus.ogg", &bytes)],
        )
        .await;
    let url = format!("http://{}/api/assets/{id}?variant=opus", h.addr);

    let head = h
        .client
        .head(&url)
        .header("accept-encoding", "gzip")
        .send()
        .await
        .unwrap();
    assert_eq!(head.status(), 200);
    assert_eq!(header(&head, "accept-ranges").as_deref(), Some("bytes"));
    assert!(header(&head, "content-encoding").is_none());
    assert_eq!(header(&head, "content-length"), Some(SIZE.to_string()));

    let unsatisfiable = fetch(&h, &url, Some(&format!("bytes={SIZE}-"))).await;
    assert_eq!(unsatisfiable.status(), 416);
    assert!(header(&unsatisfiable, "content-encoding").is_none());

    let etag = format!("\"{id}-3-opus\"");
    let revalidated = h
        .client
        .get(&url)
        .header("accept-encoding", "gzip")
        .header("if-none-match", etag)
        .send()
        .await
        .unwrap();
    assert_eq!(revalidated.status(), 304);
    assert!(header(&revalidated, "content-encoding").is_none());
}
