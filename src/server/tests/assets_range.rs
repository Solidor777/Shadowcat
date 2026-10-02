//! Byte-range behaviour of the stored-file read sites of `GET /api/assets/{uuid}`: the Opus and
//! WebM derivatives, the grid sheet, the canonical file and the `thumb`/`preview` derivatives.
//!
//! Every site goes through one streaming path, so each assertion runs over the full site table and
//! a divergence at one site fails with that site's label. Files are seeded directly (a row plus
//! bytes on disk) so a response can be larger than the 2 MiB media slice without running the
//! conversion pipeline.

use shadowcat::data::asset::process::SheetMeta;
use shadowcat::data::asset::{Asset, AssetMeta};
use shadowcat_test_support as common;
use shadowcat_test_support::Harness;
use uuid::Uuid;

/// 2 MiB: the most one media response carries.
const SLICE: usize = 2 * 1024 * 1024;

/// A seeded file larger than one slice.
const SIZE: usize = 3 * 1024 * 1024 + 17;

/// Bytes whose value depends on the offset, so a wrongly placed slice is visible.
fn pattern(len: usize) -> Vec<u8> {
    (0..len).map(|i| ((i * 7 + i / 251) % 256) as u8).collect()
}

/// Inserts an asset row of `content_type` with `meta` in the harness world, at version 3, and
/// returns its id. No file is written.
async fn insert_row(h: &Harness, content_type: &str, meta: AssetMeta) -> Uuid {
    let id = Uuid::new_v4();
    h.repo
        .insert_asset(&Asset {
            id,
            world_id: h.world,
            storage_key: format!("{}/{}", h.world, id),
            original_name: "seeded.bin".into(),
            content_type: content_type.into(),
            byte_size: 0,
            created_by: Some(h.user),
            created_at: 0,
            version: 3,
            folder_id: None,
            tags: vec![],
            derived_tags: vec![],
            meta,
        })
        .await
        .unwrap();
    id
}

/// Writes `files` (suffix, bytes) beside the canonical path of asset `id`.
fn write_files(h: &Harness, id: Uuid, files: &[(&str, &[u8])]) {
    let dir = h.assets_dir.join(h.world.to_string());
    std::fs::create_dir_all(&dir).unwrap();
    for (suffix, bytes) in files {
        std::fs::write(dir.join(format!("{id}{suffix}")), bytes).unwrap();
    }
}

/// Inserts an asset row of `content_type` in the harness world and writes `files` (suffix,
/// bytes) beside its canonical path. `with_sheet` records grid-sheet metadata on the row.
async fn seed(h: &Harness, content_type: &str, files: &[(&str, &[u8])], with_sheet: bool) -> Uuid {
    let mut meta = AssetMeta::unprocessed(content_type, 0);
    if with_sheet {
        meta.sheet = Some(SheetMeta {
            rows: 1,
            cols: 2,
            count: 2,
            frame_ms: vec![100, 100],
            width: 4,
            height: 4,
        });
    }
    let id = insert_row(h, content_type, meta).await;
    write_files(h, id, files);
    id
}

/// Seeds an asset whose retained original (`<id>.orig`) is `bytes`, arrived as
/// `original_type`, beside a one-byte canonical.
async fn seed_original(h: &Harness, original_type: &str, bytes: &[u8]) -> Uuid {
    let mut meta = AssetMeta::unprocessed("image/webp", 1);
    meta.original_retained = true;
    meta.original_content_type = original_type.into();
    meta.original_byte_size = bytes.len() as i64;
    let id = insert_row(h, "image/webp", meta).await;
    write_files(h, id, &[("", b"x"), (".orig", bytes)]);
    id
}

/// One stored-file read site.
struct Site {
    /// Names the site in assertion messages.
    label: &'static str,
    /// Asset id.
    id: Uuid,
    /// Query string selecting the site.
    query: &'static str,
    /// The `Content-Type` the site answers with.
    content_type: &'static str,
    /// The seeded bytes.
    bytes: Vec<u8>,
}

impl Site {
    fn url(&self, h: &Harness) -> String {
        format!("http://{}/api/assets/{}{}", h.addr, self.id, self.query)
    }
}

/// Seeds every media-sliced read site with the same `SIZE`-byte pattern.
async fn media_sites(h: &Harness) -> Vec<Site> {
    let bytes = pattern(SIZE);
    let mut sites = Vec::new();
    let opus = seed(h, "audio/wav", &[(".opus.ogg", &bytes)], false).await;
    sites.push(Site {
        label: "opus derivative",
        id: opus,
        query: "?variant=opus",
        content_type: "audio/ogg; codecs=opus",
        bytes: bytes.clone(),
    });
    let webm = seed(h, "audio/wav", &[(".opus.webm", &bytes)], false).await;
    sites.push(Site {
        label: "opus-webm derivative",
        id: webm,
        query: "?variant=opus-webm",
        content_type: "audio/webm; codecs=opus",
        bytes: bytes.clone(),
    });
    let sheet = seed(
        h,
        "image/webp",
        &[("", b"x"), (".sheet.webp", &bytes)],
        true,
    )
    .await;
    sites.push(Site {
        label: "grid sheet",
        id: sheet,
        query: "?variant=sheet",
        content_type: "image/webp",
        bytes: bytes.clone(),
    });
    let canonical = seed(h, "image/webp", &[("", &bytes)], false).await;
    sites.push(Site {
        label: "canonical image",
        id: canonical,
        query: "",
        content_type: "image/webp",
        bytes: bytes.clone(),
    });
    let derivatives = seed(
        h,
        "image/webp",
        &[
            ("", b"x"),
            (".thumb.webp", &bytes),
            (".preview.webp", &bytes),
        ],
        false,
    )
    .await;
    sites.push(Site {
        label: "thumb derivative",
        id: derivatives,
        query: "?variant=thumb",
        content_type: "image/webp",
        bytes: bytes.clone(),
    });
    sites.push(Site {
        label: "preview derivative",
        id: derivatives,
        query: "?variant=preview",
        content_type: "image/webp",
        bytes,
    });
    sites
}

async fn fetch(
    h: &Harness,
    url: &str,
    headers: &[(&str, String)],
    head: bool,
) -> reqwest::Response {
    let mut request = if head {
        h.client.head(url)
    } else {
        h.client.get(url)
    };
    for (name, value) in headers {
        request = request.header(*name, value);
    }
    request.send().await.unwrap()
}

fn header(res: &reqwest::Response, name: &str) -> Option<String> {
    res.headers()
        .get(name)
        .map(|v| v.to_str().unwrap().to_string())
}

/// Parses a `multipart/byteranges` body into (first, last, payload) triples, checking the framing.
fn parse_multipart(body: &[u8], boundary: &str) -> Vec<(usize, usize, Vec<u8>)> {
    let mut rest = body;
    let mut parts = Vec::new();
    loop {
        let delimiter = format!("--{boundary}");
        assert!(rest.starts_with(delimiter.as_bytes()), "delimiter expected");
        rest = &rest[delimiter.len()..];
        if rest == b"--\r\n" {
            return parts;
        }
        assert!(rest.starts_with(b"\r\n"));
        rest = &rest[2..];
        let end = rest
            .windows(4)
            .position(|w| w == b"\r\n\r\n")
            .expect("blank line ends the part headers");
        let headers = std::str::from_utf8(&rest[..end]).unwrap().to_string();
        rest = &rest[end + 4..];
        let range = headers
            .split("\r\n")
            .find_map(|l| l.strip_prefix("Content-Range: bytes "))
            .expect("Content-Range header");
        let (span, _) = range.split_once('/').unwrap();
        let (first, last) = span.split_once('-').unwrap();
        let (first, last): (usize, usize) = (first.parse().unwrap(), last.parse().unwrap());
        let n = last - first + 1;
        parts.push((first, last, rest[..n].to_vec()));
        rest = &rest[n..];
        assert!(rest.starts_with(b"\r\n"));
        rest = &rest[2..];
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_plain_get_is_a_full_200_with_an_exact_length_and_accept_ranges() {
    let h = common::spawn().await;
    for site in media_sites(&h).await {
        let res = fetch(&h, &site.url(&h), &[], false).await;
        let label = site.label;
        assert_eq!(res.status(), 200, "{label}");
        assert_eq!(
            header(&res, "accept-ranges").as_deref(),
            Some("bytes"),
            "{label}"
        );
        assert_eq!(
            header(&res, "content-length"),
            Some(SIZE.to_string()),
            "{label}"
        );
        assert_eq!(
            header(&res, "content-type").as_deref(),
            Some(site.content_type),
            "{label}"
        );
        assert_eq!(
            header(&res, "cache-control").as_deref(),
            Some("no-cache"),
            "{label}"
        );
        assert!(header(&res, "content-encoding").is_none(), "{label}");
        assert_eq!(
            res.bytes().await.unwrap().as_ref(),
            site.bytes.as_slice(),
            "{label}"
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_byte_range_is_a_206_with_exactly_the_requested_bytes() {
    let h = common::spawn().await;
    for site in media_sites(&h).await {
        let label = site.label;
        let res = fetch(&h, &site.url(&h), &[("range", "bytes=10-19".into())], false).await;
        assert_eq!(res.status(), 206, "{label}");
        assert_eq!(
            header(&res, "content-range"),
            Some(format!("bytes 10-19/{SIZE}")),
            "{label}"
        );
        assert_eq!(
            header(&res, "content-length").as_deref(),
            Some("10"),
            "{label}"
        );
        assert_eq!(
            res.bytes().await.unwrap().as_ref(),
            &site.bytes[10..20],
            "{label}"
        );

        // A suffix range reads from the end.
        let res = fetch(&h, &site.url(&h), &[("range", "bytes=-5".into())], false).await;
        assert_eq!(res.status(), 206, "{label}");
        assert_eq!(
            res.bytes().await.unwrap().as_ref(),
            &site.bytes[SIZE - 5..],
            "{label}"
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_open_ended_range_of_a_media_site_is_one_two_mib_slice() {
    let h = common::spawn().await;
    for site in media_sites(&h).await {
        let label = site.label;
        let res = fetch(&h, &site.url(&h), &[("range", "bytes=0-".into())], false).await;
        assert_eq!(res.status(), 206, "{label}");
        assert_eq!(
            header(&res, "content-range"),
            Some(format!("bytes 0-{}/{SIZE}", SLICE - 1)),
            "{label}"
        );
        assert_eq!(
            res.bytes().await.unwrap().as_ref(),
            &site.bytes[..SLICE],
            "{label}"
        );
        // The element continues from where the slice ended.
        let res = fetch(
            &h,
            &site.url(&h),
            &[("range", format!("bytes={SLICE}-"))],
            false,
        )
        .await;
        assert_eq!(res.status(), 206, "{label}");
        assert_eq!(
            header(&res, "content-range"),
            Some(format!("bytes {SLICE}-{}/{SIZE}", SIZE - 1)),
            "{label}"
        );
        assert_eq!(
            res.bytes().await.unwrap().as_ref(),
            &site.bytes[SLICE..],
            "{label}"
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_range_past_the_end_is_a_416_naming_the_length() {
    let h = common::spawn().await;
    for site in media_sites(&h).await {
        let label = site.label;
        let res = fetch(
            &h,
            &site.url(&h),
            &[("range", format!("bytes={SIZE}-"))],
            false,
        )
        .await;
        assert_eq!(res.status(), 416, "{label}");
        assert_eq!(
            header(&res, "content-range"),
            Some(format!("bytes */{SIZE}")),
            "{label}"
        );
        assert!(res.bytes().await.unwrap().is_empty(), "{label}");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn several_ranges_are_a_multipart_response_that_parses_back_exactly() {
    let h = common::spawn().await;
    for site in media_sites(&h).await {
        let label = site.label;
        let res = fetch(
            &h,
            &site.url(&h),
            &[("range", "bytes=0-9, 1000-1999,5000-5004".into())],
            false,
        )
        .await;
        assert_eq!(res.status(), 206, "{label}");
        let content_type = header(&res, "content-type").unwrap();
        let boundary = content_type
            .strip_prefix("multipart/byteranges; boundary=")
            .unwrap_or_else(|| panic!("{label}: {content_type}"))
            .to_string();
        let declared: usize = header(&res, "content-length").unwrap().parse().unwrap();
        let body = res.bytes().await.unwrap();
        assert_eq!(body.len(), declared, "{label}: exact Content-Length");
        let parts = parse_multipart(&body, &boundary);
        let spans: Vec<(usize, usize)> = parts.iter().map(|(a, b, _)| (*a, *b)).collect();
        assert_eq!(spans, vec![(0, 9), (1000, 1999), (5000, 5004)], "{label}");
        for (first, last, payload) in parts {
            assert_eq!(payload, &site.bytes[first..=last], "{label}");
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn revalidation_is_answered_before_the_range_is_considered() {
    let h = common::spawn().await;
    for site in media_sites(&h).await {
        let label = site.label;
        let etag = header(&fetch(&h, &site.url(&h), &[], false).await, "etag").unwrap();
        let res = fetch(
            &h,
            &site.url(&h),
            &[
                ("if-none-match", etag.clone()),
                ("range", "bytes=10-19".into()),
            ],
            false,
        )
        .await;
        assert_eq!(res.status(), 304, "{label}");
        assert_eq!(
            header(&res, "etag").as_deref(),
            Some(etag.as_str()),
            "{label}"
        );
        assert!(header(&res, "content-range").is_none(), "{label}");
        assert!(header(&res, "accept-ranges").is_none(), "{label}");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn if_range_honours_the_range_only_for_the_current_validator() {
    let h = common::spawn().await;
    for site in media_sites(&h).await {
        let label = site.label;
        let etag = header(&fetch(&h, &site.url(&h), &[], false).await, "etag").unwrap();
        let matching = fetch(
            &h,
            &site.url(&h),
            &[("range", "bytes=10-19".into()), ("if-range", etag.clone())],
            false,
        )
        .await;
        assert_eq!(matching.status(), 206, "{label}");
        let stale = fetch(
            &h,
            &site.url(&h),
            &[
                ("range", "bytes=10-19".into()),
                ("if-range", "\"stale\"".into()),
            ],
            false,
        )
        .await;
        assert_eq!(stale.status(), 200, "{label}");
        assert_eq!(stale.bytes().await.unwrap().len(), SIZE, "{label}");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn head_answers_the_status_and_headers_of_the_matching_get() {
    let h = common::spawn().await;
    for site in media_sites(&h).await {
        let label = site.label;
        for range in [
            None,
            Some("bytes=10-19"),
            Some("bytes=0-"),
            Some(format!("bytes={SIZE}-").leak() as &str),
            Some("bytes=0-9,100-199"),
        ] {
            let request: Vec<(&str, String)> = range
                .map(|r| vec![("range", r.to_string())])
                .unwrap_or_default();
            let got = fetch(&h, &site.url(&h), &request, false).await;
            let head = fetch(&h, &site.url(&h), &request, true).await;
            assert_eq!(got.status(), head.status(), "{label} {range:?}");
            for name in [
                "content-length",
                "content-range",
                "accept-ranges",
                "etag",
                "cache-control",
            ] {
                assert_eq!(
                    header(&got, name),
                    header(&head, name),
                    "{label} {range:?} {name}"
                );
            }
            // The boundary of a multipart answer is fresh per response.
            if !header(&got, "content-type")
                .unwrap_or_default()
                .starts_with("multipart/")
            {
                assert_eq!(
                    header(&got, "content-type"),
                    header(&head, "content-type"),
                    "{label} {range:?}"
                );
            }
            assert!(head.bytes().await.unwrap().is_empty(), "{label} {range:?}");
            let _ = got.bytes().await.unwrap();
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_non_member_with_a_range_is_refused_before_any_file_access() {
    let h = common::spawn().await;
    let other = h.repo.create_world("Other", 0).await.unwrap();
    let id = Uuid::new_v4();
    h.repo
        .insert_asset(&Asset {
            id,
            world_id: other.id,
            storage_key: format!("{}/{id}", other.id),
            original_name: "x.bin".into(),
            content_type: "image/webp".into(),
            byte_size: 1,
            created_by: Some(h.user),
            created_at: 0,
            version: 1,
            folder_id: None,
            tags: vec![],
            derived_tags: vec![],
            meta: AssetMeta::unprocessed("image/webp", 1),
        })
        .await
        .unwrap();
    for query in ["", "?variant=opus", "?variant=thumb"] {
        let res = fetch(
            &h,
            &format!("http://{}/api/assets/{id}{query}", h.addr),
            &[("range", "bytes=0-9".into())],
            false,
        )
        .await;
        assert_eq!(res.status(), 403, "{query}");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_missing_file_keeps_its_status_per_site() {
    let h = common::spawn().await;
    let opus = seed(&h, "audio/wav", &[], false).await;
    let sheet = seed(&h, "image/webp", &[], true).await;
    let canonical = seed(&h, "image/webp", &[], false).await;
    for (label, id, query, status) in [
        ("opus", opus, "?variant=opus", 404),
        ("sheet", sheet, "?variant=sheet", 404),
        ("canonical", canonical, "", 500),
    ] {
        for range in [None, Some("bytes=0-9")] {
            let request: Vec<(&str, String)> = range
                .map(|r| vec![("range", r.to_string())])
                .unwrap_or_default();
            let res = fetch(
                &h,
                &format!("http://{}/api/assets/{id}{query}", h.addr),
                &request,
                false,
            )
            .await;
            assert_eq!(res.status(), status, "{label} {range:?}");
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_attachment_download_is_uncapped_and_a_compressible_type_ignores_range() {
    let h = common::spawn().await;
    let bytes = pattern(SIZE);
    // `audio/wav` is a ranged type served as an attachment: whole-file semantics, no 2 MiB cap.
    let wav = seed(&h, "audio/wav", &[("", &bytes)], false).await;
    let res = fetch(
        &h,
        &format!("http://{}/api/assets/{wav}", h.addr),
        &[("range", "bytes=0-".into())],
        false,
    )
    .await;
    assert_eq!(res.status(), 206);
    assert_eq!(
        header(&res, "content-range"),
        Some(format!("bytes 0-{}/{SIZE}", SIZE - 1))
    );
    assert_eq!(res.bytes().await.unwrap().as_ref(), bytes.as_slice());
    // `text/plain` is compressible: no `Accept-Ranges`, `Range` ignored, whole body.
    let text = seed(&h, "text/plain", &[("", &bytes)], false).await;
    let res = fetch(
        &h,
        &format!("http://{}/api/assets/{text}", h.addr),
        &[("range", "bytes=10-19".into())],
        false,
    )
    .await;
    assert_eq!(res.status(), 200);
    assert!(header(&res, "accept-ranges").is_none());
    assert!(header(&res, "content-range").is_none());
    assert_eq!(res.bytes().await.unwrap().as_ref(), bytes.as_slice());
}

/// A validator hit costs no file access: the answer is the same with the file absent.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_validator_hit_is_answered_without_opening_the_file() {
    let h = common::spawn().await;
    let opus = seed(&h, "audio/wav", &[], false).await;
    let sheet = seed(&h, "image/webp", &[], true).await;
    let canonical = seed(&h, "image/webp", &[], false).await;
    for (label, id, query, etag) in [
        ("opus", opus, "?variant=opus", format!("\"{opus}-3-opus\"")),
        ("sheet", sheet, "?variant=sheet", format!("\"{sheet}-3\"")),
        ("canonical", canonical, "", format!("\"{canonical}-3\"")),
    ] {
        let res = fetch(
            &h,
            &format!("http://{}/api/assets/{id}{query}", h.addr),
            &[("if-none-match", etag)],
            false,
        )
        .await;
        assert_eq!(res.status(), 304, "{label}");
    }
}

/// The retained-original download is a ranged, uncompressed, uncapped download of every type, so a
/// resume `bytes=N-` is the rest of the file in one response.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_retained_original_is_a_resumable_download_of_any_type() {
    let h = common::spawn().await;
    let bytes = pattern(SIZE);
    for original_type in ["application/octet-stream", "text/plain", "image/png"] {
        let id = seed_original(&h, original_type, &bytes).await;
        let url = format!("http://{}/api/assets/{id}/original", h.addr);
        let res = fetch(&h, &url, &[], false).await;
        assert_eq!(res.status(), 200, "{original_type}");
        assert_eq!(header(&res, "accept-ranges").as_deref(), Some("bytes"));
        assert!(
            header(&res, "content-encoding").is_none(),
            "{original_type}"
        );
        assert_eq!(
            header(&res, "content-length"),
            Some(SIZE.to_string()),
            "{original_type}"
        );
        assert!(header(&res, "content-disposition")
            .unwrap()
            .starts_with("attachment; filename="));
        assert_eq!(header(&res, "content-type").as_deref(), Some(original_type));
        assert_eq!(header(&res, "etag"), Some(format!("\"{id}-3-orig\"")));
        assert_eq!(res.bytes().await.unwrap().as_ref(), bytes.as_slice());

        let res = fetch(&h, &url, &[("range", "bytes=5-9".into())], false).await;
        assert_eq!(res.status(), 206, "{original_type}");
        assert_eq!(res.bytes().await.unwrap().as_ref(), &bytes[5..10]);

        let resume = 100;
        let res = fetch(&h, &url, &[("range", format!("bytes={resume}-"))], false).await;
        assert_eq!(res.status(), 206, "{original_type}");
        assert_eq!(
            header(&res, "content-range"),
            Some(format!("bytes {resume}-{}/{SIZE}", SIZE - 1))
        );
        assert_eq!(res.bytes().await.unwrap().as_ref(), &bytes[resume..]);
    }
}

/// The original's validator order: a matching `If-None-Match` answers 304 before the file is
/// touched, a stale `If-Range` yields the whole file, and HEAD answers the GET's headers.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_retained_original_honours_validators_and_head() {
    let h = common::spawn().await;
    let bytes = pattern(4096);
    let id = seed_original(&h, "application/octet-stream", &bytes).await;
    let url = format!("http://{}/api/assets/{id}/original", h.addr);
    let etag = format!("\"{id}-3-orig\"");

    let res = fetch(
        &h,
        &url,
        &[
            ("if-none-match", etag.clone()),
            ("range", "bytes=0-9".into()),
        ],
        false,
    )
    .await;
    assert_eq!(res.status(), 304);
    assert!(header(&res, "content-range").is_none());

    let res = fetch(
        &h,
        &url,
        &[
            ("range", "bytes=0-9".into()),
            ("if-range", "\"stale\"".into()),
        ],
        false,
    )
    .await;
    assert_eq!(res.status(), 200);
    assert_eq!(res.bytes().await.unwrap().len(), 4096);

    let res = fetch(
        &h,
        &url,
        &[("range", "bytes=0-9".into()), ("if-range", etag)],
        false,
    )
    .await;
    assert_eq!(res.status(), 206);

    let res = fetch(&h, &url, &[("range", "bytes=10-19".into())], true).await;
    assert_eq!(res.status(), 206);
    assert_eq!(
        header(&res, "content-range").as_deref(),
        Some("bytes 10-19/4096")
    );
    assert!(res.bytes().await.unwrap().is_empty());
}

/// The original keeps its GM gate when a `Range` is present, and a not-retained original is a 404.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_retained_original_keeps_its_gm_gate_and_its_missing_statuses() {
    let h = common::spawn().await;
    let id = seed_original(&h, "application/octet-stream", &pattern(64)).await;
    let url = format!("http://{}/api/assets/{id}/original", h.addr);
    let (_pid, cookie) = h.add_player("range-viewer").await;
    let res = reqwest::Client::new()
        .get(&url)
        .header("cookie", &cookie)
        .header("range", "bytes=0-9")
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 403);

    let not_retained = seed(&h, "image/webp", &[("", b"x")], false).await;
    let res = fetch(
        &h,
        &format!("http://{}/api/assets/{not_retained}/original", h.addr),
        &[("range", "bytes=0-9".into())],
        false,
    )
    .await;
    assert_eq!(res.status(), 404);

    let mut meta = AssetMeta::unprocessed("image/webp", 1);
    meta.original_retained = true;
    let absent = insert_row(&h, "image/webp", meta).await;
    let res = fetch(
        &h,
        &format!("http://{}/api/assets/{absent}/original", h.addr),
        &[],
        false,
    )
    .await;
    assert_eq!(res.status(), 500);
}
