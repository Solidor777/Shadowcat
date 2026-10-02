//! Streamed whole-file responses with byte-range support, shared by every asset site that serves a
//! stored file, so the validator order, the range decision and the response framing cannot differ
//! between sites.
//!
//! The file is read in `FILE_READ_CHUNK` pieces as the connection pulls them; no response holds a
//! file's bytes in memory. `http::conn_serve`'s `LeasedBody` re-chunks every response body to the
//! egress frame size, so the read size is chosen for syscall count and is not coupled to it.
//!
//! A GET holds one `FilePermit` for as long as it holds the file open. The permit is taken before
//! the file is opened and before any header is written, so an exhausted pool is a clean 503, and
//! it moves into the body, which returns it when the last file byte has been read or when the
//! body is dropped (a peer that reset, or a connection cut by the egress stall bounds).
#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use std::collections::VecDeque;
use std::io::SeekFrom;
use std::path::Path;

use axum::body::{Body, Bytes};
use axum::http::{header, HeaderMap, HeaderValue, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use tokio::fs::File;
use tokio::io::{AsyncReadExt, AsyncSeekExt};
use uuid::Uuid;

use super::permits::{FilePermit, StreamedFilePermits};
use super::range::{
    classify, decide, multipart_close, multipart_content_type, multipart_len, multipart_part_head,
    new_boundary, Disposition, RangeClass, RangeDecision, Route, Slice, PART_END,
};
use crate::http::error::AppError;

/// Bytes read from the file per body chunk.
pub(super) const FILE_READ_CHUNK: usize = 64 * 1024;

/// How an absent file is reported, which differs by site: a missing derivative is an ordinary 404,
/// a missing canonical for an existing record is a server fault.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum MissingFile {
    /// 404.
    NotFound,
    /// 500, with the open error logged.
    Internal,
}

/// Everything `serve_file` needs to answer for one stored file.
pub(super) struct FileResponse<'a> {
    /// The asset id, named in the log line of an open failure.
    pub(super) id: Uuid,
    /// The file to stream.
    pub(super) path: &'a Path,
    /// The representation's `Content-Type`.
    pub(super) content_type: String,
    /// The full `Content-Disposition` header value.
    pub(super) content_disposition: String,
    /// The disposition type `content_disposition` carries, which `classify` reads.
    pub(super) disposition: Disposition,
    /// The route the response is served from.
    pub(super) route: Route,
    /// The strong validator, with its quotes.
    pub(super) etag: String,
    /// How an absent file is reported.
    pub(super) missing: MissingFile,
}

/// The 304 answer when `If-None-Match` lists `etag`, else `None`. `If-None-Match` is a comma
/// separated list (a browser may send several cached validators), so the match is against any
/// member. The 304 carries the validator and `Cache-Control` and no range headers.
pub(super) fn not_modified(headers: &HeaderMap, etag: &str) -> Option<Response> {
    let listed = headers
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if !listed.split(',').any(|t| t.trim() == etag) {
        return None;
    }
    let mut response = StatusCode::NOT_MODIFIED.into_response();
    response
        .headers_mut()
        .insert(header::ETAG, HeaderValue::from_str(etag).ok()?);
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    Some(response)
}

/// A header value read as text; a present header that is not text is returned as the empty
/// string, which never validates an `If-Range` and never parses as a `Range`.
fn header_text(headers: &HeaderMap, name: header::HeaderName) -> Option<&str> {
    headers.get(name).map(|v| v.to_str().unwrap_or(""))
}

/// Builds a header value from text, or reports a server fault.
fn value(text: &str) -> Result<HeaderValue, AppError> {
    HeaderValue::from_str(text).map_err(|_| AppError::Internal)
}

/// One piece of a response body.
enum Segment {
    /// Literal bytes: a multipart part head, a part terminator or the close delimiter.
    Literal(Bytes),
    /// `len` bytes of the file from offset `start`.
    File {
        /// Offset of the first byte.
        start: u64,
        /// Number of bytes.
        len: u64,
    },
}

/// The read state of a streamed body: the open file and its permit, the pieces still to send and
/// the bytes left in the file piece being read.
struct BodyState {
    /// The open file; `None` once the last file byte has been read, and dropped with the state
    /// when the peer's connection drops the body first.
    file: Option<File>,
    /// The permit the open file holds; released together with `file`.
    permit: Option<FilePermit>,
    /// Pieces not yet started.
    segments: VecDeque<Segment>,
    /// Bytes still to read from the current file piece.
    remaining: u64,
}

impl BodyState {
    /// Closes the file and returns its permit once no file byte is left to read. The body may
    /// still owe literal bytes (a multipart delimiter) and the connection may still hold the
    /// chunks already read, but neither needs the descriptor, so the permit does not stay held
    /// while a peer drains buffered bytes.
    fn release_when_read(&mut self) {
        let file_bytes_left = self.remaining > 0
            || self
                .segments
                .iter()
                .any(|s| matches!(s, Segment::File { len, .. } if *len > 0));
        if !file_bytes_left {
            self.file = None;
            self.permit = None;
        }
    }

    /// The next chunk, or `None` at the end of the body. A file that ends before the piece does
    /// (it shrank under the response) is an error, which aborts the response.
    async fn next_chunk(&mut self) -> std::io::Result<Option<Bytes>> {
        loop {
            if self.remaining > 0 {
                let want = self.remaining.min(FILE_READ_CHUNK as u64) as usize;
                let mut buffer = vec![0u8; want];
                let file = self
                    .file
                    .as_mut()
                    .ok_or(std::io::ErrorKind::UnexpectedEof)?;
                let read = file.read(&mut buffer).await?;
                if read == 0 {
                    return Err(std::io::ErrorKind::UnexpectedEof.into());
                }
                buffer.truncate(read);
                self.remaining -= read as u64;
                self.release_when_read();
                return Ok(Some(Bytes::from(buffer)));
            }
            match self.segments.pop_front() {
                None => return Ok(None),
                Some(Segment::Literal(bytes)) => {
                    self.release_when_read();
                    return Ok(Some(bytes));
                }
                Some(Segment::File { start, len }) => {
                    if len > 0 {
                        let file = self
                            .file
                            .as_mut()
                            .ok_or(std::io::ErrorKind::UnexpectedEof)?;
                        file.seek(SeekFrom::Start(start)).await?;
                        self.remaining = len;
                    }
                }
            }
        }
    }
}

/// A body that streams `segments` from `file`, holding `permit` while the file is open.
fn stream_body(file: File, permit: FilePermit, segments: VecDeque<Segment>) -> Body {
    let mut state = BodyState {
        file: Some(file),
        permit: Some(permit),
        segments,
        remaining: 0,
    };
    state.release_when_read();
    Body::from_stream(futures_util::stream::unfold(
        Some(state),
        |state| async move {
            let mut state = state?;
            match state.next_chunk().await {
                Ok(Some(chunk)) => Some((Ok(chunk), Some(state))),
                Ok(None) => None,
                Err(e) => Some((Err(e), None)),
            }
        },
    ))
}

/// Builds the response for `decision` over a representation of `len` bytes. `open` is the open
/// handle of a GET with the permit that covers it, and `None` for a HEAD, whose response is the
/// same status and headers with an empty body; the one pure `decide` and this one builder serve
/// both edges. A response without a file body (a 416) drops `open`, returning the permit.
fn respond(
    spec: &FileResponse<'_>,
    class: RangeClass,
    decision: RangeDecision,
    len: u64,
    open: Option<(File, FilePermit)>,
) -> Result<Response, AppError> {
    let mut headers = HeaderMap::new();
    headers.insert(header::ETAG, value(&spec.etag)?);
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    if class != RangeClass::Ignore {
        headers.insert(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));
    }
    let representation_type = value(&spec.content_type)?;
    let (status, content_length, segments) = match decision {
        RangeDecision::Unsatisfiable => {
            headers.insert(header::CONTENT_RANGE, value(&format!("bytes */{len}"))?);
            let mut response = Response::new(Body::empty());
            *response.status_mut() = StatusCode::RANGE_NOT_SATISFIABLE;
            headers.insert(header::CONTENT_LENGTH, HeaderValue::from(0u64));
            *response.headers_mut() = headers;
            return Ok(response);
        }
        RangeDecision::Full => {
            headers.insert(header::CONTENT_TYPE, representation_type);
            headers.insert(
                header::CONTENT_DISPOSITION,
                value(&spec.content_disposition)?,
            );
            (
                StatusCode::OK,
                len,
                VecDeque::from([Segment::File { start: 0, len }]),
            )
        }
        RangeDecision::Slices(slices) => match slices.as_slice() {
            [slice] => {
                headers.insert(header::CONTENT_TYPE, representation_type);
                headers.insert(
                    header::CONTENT_DISPOSITION,
                    value(&spec.content_disposition)?,
                );
                headers.insert(
                    header::CONTENT_RANGE,
                    value(&format!("bytes {}-{}/{len}", slice.start, slice.end))?,
                );
                (
                    StatusCode::PARTIAL_CONTENT,
                    slice.len(),
                    VecDeque::from([Segment::File {
                        start: slice.start,
                        len: slice.len(),
                    }]),
                )
            }
            _ => {
                let boundary = new_boundary();
                headers.insert(
                    header::CONTENT_TYPE,
                    value(&multipart_content_type(&boundary))?,
                );
                headers.insert(
                    header::CONTENT_DISPOSITION,
                    value(&spec.content_disposition)?,
                );
                (
                    StatusCode::PARTIAL_CONTENT,
                    multipart_len(&boundary, &spec.content_type, &slices, len),
                    multipart_segments(&boundary, &spec.content_type, &slices, len),
                )
            }
        },
    };
    headers.insert(header::CONTENT_LENGTH, HeaderValue::from(content_length));
    let body = match open {
        Some((file, permit)) => stream_body(file, permit, segments),
        None => Body::empty(),
    };
    let mut response = Response::new(body);
    *response.status_mut() = status;
    *response.headers_mut() = headers;
    Ok(response)
}

/// The body pieces of a `multipart/byteranges` response: each part's head, its file bytes and its
/// terminator, then the close delimiter.
fn multipart_segments(
    boundary: &str,
    content_type: &str,
    slices: &[Slice],
    len: u64,
) -> VecDeque<Segment> {
    let mut segments = VecDeque::new();
    for slice in slices {
        segments.push_back(Segment::Literal(Bytes::from(multipart_part_head(
            boundary,
            content_type,
            *slice,
            len,
        ))));
        segments.push_back(Segment::File {
            start: slice.start,
            len: slice.len(),
        });
        segments.push_back(Segment::Literal(Bytes::from_static(PART_END.as_bytes())));
    }
    segments.push_back(Segment::Literal(Bytes::from(multipart_close(boundary))));
    segments
}

/// Answers a request for the stored file `spec` describes: the range decision over the file's
/// length, then the matching 200, 206 or 416 with the body streamed from the file.
///
/// A GET takes a permit from `permits` (a bounded wait, then 503), opens the file and takes the
/// length from that handle, so the length, the validator the caller computed and the bytes
/// streamed describe one file even when a replace renames over the name mid-request. A HEAD reads
/// the length from the file's metadata, takes no permit and opens nothing.
pub(super) async fn serve_file(
    spec: FileResponse<'_>,
    headers: &HeaderMap,
    method: &Method,
    permits: &StreamedFilePermits,
) -> Result<Response, AppError> {
    let class = classify(&spec.content_type, spec.disposition, spec.route);
    let missing = |e: std::io::Error| {
        if spec.missing == MissingFile::Internal {
            tracing::error!(?e, id = %spec.id, "asset file missing for existing record");
            AppError::Internal
        } else {
            tracing::debug!(?e, id = %spec.id, "asset file missing");
            AppError::NotFound
        }
    };
    let (len, open) = if *method == Method::HEAD {
        let metadata = tokio::fs::metadata(spec.path).await.map_err(missing)?;
        (metadata.len(), None)
    } else {
        let permit = permits.acquire().await?;
        let file = File::open(spec.path).await.map_err(missing)?;
        let len = file.metadata().await.map_err(|_| AppError::Internal)?.len();
        (len, Some((file, permit)))
    };
    let decision = decide(
        class,
        header_text(headers, header::RANGE),
        header_text(headers, header::IF_RANGE),
        &spec.etag,
        len,
    );
    respond(&spec, class, decision, len, open)
}

#[cfg(test)]
mod tests;
