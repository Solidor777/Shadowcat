//! Streamed asset files over real sockets, HTTP/1 and HTTP/2 with prior knowledge, with the
//! egress no-progress window at one second.
//!
//! What these pin: a response whose peer stops reading is cut, and the cut returns the streamed-file
//! permit and closes the file handle; a media element that reads one slice per request and pauses
//! between requests never leaves a response in flight past the window; a cut is recoverable
//! through `Range`; a compressible full response stalls and is cut like any body; a drain with a
//! response open ends within the drain bound; a world export is bounded by the same permits.
//!
//! Cut counts are read from the `DrainReport` the serve loop returns at shutdown. A permit is
//! released when its response's body is dropped, which is also when the file closes, so the
//! tests read both: the permit pool's `held()` and whether the file is still open in this
//! process.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use axum::body::Bytes;
use shadowcat::auth::password::hash_password;
use shadowcat::auth::role::ServerRole;
use shadowcat::config::Config;
use shadowcat::data::asset::{Asset, AssetMeta};
use shadowcat::http::assets::permits::StreamedFilePermits;
use shadowcat::http::conn_serve::{ConnLimits, DrainReport, StallCuts, ABORT_WAIT};
use shadowcat_test_support::egress::{EgressServer, Protocol};
use shadowcat_test_support::h2_session::{login, seeded_state};
use shadowcat_test_support::tls::H2Client;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::Instant;
use uuid::Uuid;

/// The no-progress window the tests run at.
const WINDOW: Duration = Duration::from_secs(1);

/// The sampler period at [`WINDOW`]: a quarter of it.
const TICK: Duration = Duration::from_millis(250);

/// The drain bound the tests run at.
const DRAIN: Duration = Duration::from_millis(500);

/// How long a request waits for a permit in these tests.
const PERMIT_WAIT: Duration = Duration::from_millis(300);

/// One media slice: the most a ranged inline response carries.
const SLICE: usize = 2 * 1024 * 1024;

/// Size of the seeded media file: six slices, larger than any buffer between server and client.
const MEDIA_LEN: usize = 6 * SLICE;

/// Slack added to a cut's expected time for scheduling.
const SLACK: Duration = Duration::from_millis(1500);

/// Bytes whose value depends on the offset, so a wrongly placed slice is visible.
fn pattern(len: usize) -> Vec<u8> {
    (0..len).map(|i| ((i * 7 + i / 251) % 256) as u8).collect()
}

/// Printable pseudo-random bytes that gzip only to about three quarters of their size, so a
/// compressed response of them is far larger than the buffers between server and client.
fn incompressible_text(len: usize) -> Vec<u8> {
    let alphabet = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789 \n";
    let mut state = 0x9E37_79B9_7F4A_7C15_u64;
    (0..len)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            alphabet[(state % alphabet.len() as u64) as usize]
        })
        .collect()
}

/// Whether this process holds `path` open. Windows probes with an exclusive open, which a held
/// handle (std opens with read, write and delete sharing) refuses with a sharing violation; Linux
/// reads the process's descriptor table; macOS asks each descriptor for its path.
fn file_is_open(path: &Path) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        /// `ERROR_SHARING_VIOLATION`.
        const SHARING_VIOLATION: i32 = 32;
        match std::fs::OpenOptions::new()
            .write(true)
            .share_mode(0)
            .open(path)
        {
            Ok(_) => false,
            Err(e) => {
                assert_eq!(e.raw_os_error(), Some(SHARING_VIOLATION), "{e}");
                true
            }
        }
    }
    #[cfg(target_os = "linux")]
    {
        let want = std::fs::canonicalize(path).unwrap();
        std::fs::read_dir("/proc/self/fd")
            .unwrap()
            .filter_map(Result::ok)
            .any(|entry| std::fs::read_link(entry.path()).is_ok_and(|target| target == want))
    }
    #[cfg(target_os = "macos")]
    {
        let want = std::fs::canonicalize(path).unwrap();
        (0..4096).any(|fd| {
            let mut buf = [0 as libc::c_char; libc::PATH_MAX as usize];
            // SAFETY: `buf` is PATH_MAX bytes, the size F_GETPATH requires; an invalid
            // descriptor makes the call return -1 and write nothing.
            let rc = unsafe { libc::fcntl(fd, libc::F_GETPATH, buf.as_mut_ptr()) };
            rc != -1 && {
                let text = unsafe { std::ffi::CStr::from_ptr(buf.as_ptr()) };
                Path::new(text.to_str().unwrap()) == want
            }
        })
    }
}

/// Polls `condition` every 20 ms until it holds or `limit` passes; whether it held.
async fn wait_for(limit: Duration, mut condition: impl FnMut() -> bool) -> bool {
    let start = Instant::now();
    loop {
        if condition() {
            return true;
        }
        if start.elapsed() >= limit {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// A response the test reads at its own pace.
enum Reply {
    /// An HTTP/1.1 response read from its own connection.
    H1 {
        /// The status code.
        status: u16,
        /// Lower-cased header names to values.
        headers: HashMap<String, String>,
        /// The connection, positioned after what `leftover` holds.
        stream: TcpStream,
        /// Body bytes read together with the head.
        leftover: Vec<u8>,
        /// Content-Length bytes not yet returned, when the response has a length.
        remaining: Option<u64>,
    },
    /// An HTTP/2 response.
    H2 {
        /// The status code.
        status: u16,
        /// Lower-cased header names to values.
        headers: HashMap<String, String>,
        /// The body.
        body: h2::RecvStream,
    },
}

/// The connection ended before the body did.
#[derive(Debug)]
struct Cut;

impl Reply {
    /// The status code.
    fn status(&self) -> u16 {
        match self {
            Reply::H1 { status, .. } | Reply::H2 { status, .. } => *status,
        }
    }

    /// A header value by lower-case name.
    fn header(&self, name: &str) -> Option<&str> {
        match self {
            Reply::H1 { headers, .. } | Reply::H2 { headers, .. } => {
                headers.get(name).map(String::as_str)
            }
        }
    }

    /// The next body bytes, `None` at the end of the body, `Err(Cut)` if the connection or stream
    /// ended first. An HTTP/2 read returns its credit to the sender.
    async fn read_some(&mut self) -> Result<Option<Bytes>, Cut> {
        match self {
            Reply::H1 {
                stream,
                leftover,
                remaining,
                ..
            } => {
                if remaining == &Some(0) {
                    return Ok(None);
                }
                if !leftover.is_empty() {
                    let bytes = Bytes::from(std::mem::take(leftover));
                    if let Some(left) = remaining {
                        *left -= bytes.len() as u64;
                    }
                    return Ok(Some(bytes));
                }
                let mut buf = vec![0u8; 64 * 1024];
                match stream.read(&mut buf).await {
                    Ok(0) | Err(_) if remaining.is_some() => Err(Cut),
                    Ok(0) | Err(_) => Ok(None),
                    Ok(n) => {
                        buf.truncate(n);
                        if let Some(left) = remaining {
                            *left -= n as u64;
                        }
                        Ok(Some(Bytes::from(buf)))
                    }
                }
            }
            Reply::H2 { body, .. } => match body.data().await {
                Some(Ok(chunk)) => {
                    let _ = body.flow_control().release_capacity(chunk.len());
                    Ok(Some(chunk))
                }
                Some(Err(_)) => Err(Cut),
                None => Ok(None),
            },
        }
    }

    /// Reads the whole body, or returns what was read when the response was cut.
    async fn read_all(&mut self) -> Result<Vec<u8>, Cut> {
        let mut all = Vec::new();
        while let Some(chunk) = self.read_some().await? {
            all.extend_from_slice(&chunk);
        }
        Ok(all)
    }

    /// Reads at least `want` body bytes (or the whole body if shorter) and returns them.
    async fn read_at_least(&mut self, want: usize) -> Result<Vec<u8>, Cut> {
        let mut got = Vec::new();
        while got.len() < want {
            match self.read_some().await? {
                Some(chunk) => got.extend_from_slice(&chunk),
                None => break,
            }
        }
        Ok(got)
    }
}

/// Sends one HTTP/1.1 request on a fresh connection and reads the response head.
async fn h1_request(
    addr: std::net::SocketAddr,
    method: &str,
    path: &str,
    headers: &[(&str, String)],
    body: &[u8],
) -> Reply {
    let mut stream = TcpStream::connect(addr).await.unwrap();
    stream.set_nodelay(true).unwrap();
    let mut request = format!("{method} {path} HTTP/1.1\r\nHost: test\r\nConnection: close\r\n");
    for (name, value) in headers {
        request.push_str(&format!("{name}: {value}\r\n"));
    }
    request.push_str(&format!("Content-Length: {}\r\n\r\n", body.len()));
    stream.write_all(request.as_bytes()).await.unwrap();
    stream.write_all(body).await.unwrap();
    let mut head = Vec::new();
    let end = loop {
        let mut buf = [0u8; 4096];
        let n = stream.read(&mut buf).await.unwrap();
        assert!(n > 0, "the connection closed before a response head");
        head.extend_from_slice(&buf[..n]);
        if let Some(at) = head.windows(4).position(|w| w == b"\r\n\r\n") {
            break at;
        }
    };
    let leftover = head.split_off(end + 4);
    let text = String::from_utf8(head).unwrap();
    let mut lines = text.split("\r\n");
    let status = lines
        .next()
        .unwrap()
        .split(' ')
        .nth(1)
        .unwrap()
        .parse()
        .unwrap();
    let mut headers = HashMap::new();
    for line in lines.filter(|l| !l.is_empty()) {
        let (name, value) = line.split_once(':').unwrap();
        headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_string());
    }
    let remaining = headers
        .get("content-length")
        .map(|v| v.parse::<u64>().unwrap());
    Reply::H1 {
        status,
        headers,
        stream,
        leftover,
        remaining,
    }
}

/// Sends one HTTP/2 request on `client` and reads the response head.
async fn h2_request(
    client: &mut H2Client,
    addr: std::net::SocketAddr,
    method: &str,
    path: &str,
    headers: &[(&str, String)],
) -> Reply {
    client.sender = client.sender.clone().ready().await.unwrap();
    let mut request = axum::http::Request::builder()
        .method(method)
        .uri(format!("http://{addr}{path}"));
    for (name, value) in headers {
        request = request.header(*name, value);
    }
    let (response, _) = client
        .sender
        .send_request(request.body(()).unwrap(), true)
        .unwrap();
    let response = response.await.unwrap();
    let status = response.status().as_u16();
    let headers = response
        .headers()
        .iter()
        .map(|(k, v)| (k.as_str().to_string(), v.to_str().unwrap().to_string()))
        .collect();
    Reply::H2 {
        status,
        headers,
        body: response.into_body(),
    }
}

/// A server over the real router with an assets directory, a small permit pool and a session.
struct Media {
    /// The serve loop.
    server: EgressServer,
    /// The assets directory; removed when the fixture drops.
    dir: tempfile::TempDir,
    /// The protocol the connections speak.
    protocol: Protocol,
    /// The HTTP/2 connection every `get` shares.
    h2: Option<H2Client>,
    /// A session cookie of the world's GM.
    cookie: String,
}

impl Media {
    /// Starts a server of `protocol` whose permit pool holds `permits` permits.
    async fn start(protocol: Protocol, permits: usize) -> Self {
        let (mut state, world) = seeded_state().await;
        let dir = tempfile::tempdir().unwrap();
        state.config = Arc::new(Config {
            assets_dir: Some(dir.path().to_string_lossy().into_owned()),
            ..Config::default()
        });
        state.file_permits = Arc::new(StreamedFilePermits::new(permits, PERMIT_WAIT));
        let server = EgressServer::start_over(
            state,
            world,
            ConnLimits {
                egress_window: WINDOW,
                drain_timeout: DRAIN,
                ..ConnLimits::default()
            },
            protocol,
            true,
        )
        .await;
        let mut media = Self {
            server,
            dir,
            protocol,
            h2: None,
            cookie: String::new(),
        };
        media.cookie = media.login("u").await;
        media
    }

    /// Logs `username` (password `pw`) in over this fixture's protocol and returns the cookie.
    async fn login(&mut self, username: &str) -> String {
        let addr = self.server.addr;
        match self.protocol {
            Protocol::H1 => {
                let body = format!(r#"{{"username":"{username}","password":"pw"}}"#);
                let reply = h1_request(
                    addr,
                    "POST",
                    "/api/login",
                    &[("content-type", "application/json".to_string())],
                    body.as_bytes(),
                )
                .await;
                assert!(
                    reply.status() == 200 || reply.status() == 204,
                    "login: {}",
                    reply.status()
                );
                reply
                    .header("set-cookie")
                    .expect("login sets a cookie")
                    .split(';')
                    .next()
                    .unwrap()
                    .to_string()
            }
            Protocol::H2 => {
                assert_eq!(
                    username, "u",
                    "the h2 login helper signs in the seeded user"
                );
                let mut client = self.server.connect(1 << 20, 1 << 22).await;
                let cookie = login(&mut client).await;
                self.h2 = Some(client);
                cookie
            }
        }
    }

    /// `GET path` with the session cookie and `headers`.
    async fn get(&mut self, path: &str, headers: &[(&str, String)]) -> Reply {
        self.send("GET", path, headers).await
    }

    /// `method path` with the session cookie and `headers`.
    async fn send(&mut self, method: &str, path: &str, headers: &[(&str, String)]) -> Reply {
        let mut all = vec![("cookie", self.cookie.clone())];
        all.extend(headers.iter().map(|(n, v)| (*n, v.clone())));
        match self.protocol {
            Protocol::H1 => h1_request(self.server.addr, method, path, &all, &[]).await,
            Protocol::H2 => {
                let addr = self.server.addr;
                h2_request(self.h2.as_mut().unwrap(), addr, method, path, &all).await
            }
        }
    }

    /// The pool the handlers draw from.
    fn permits(&self) -> &StreamedFilePermits {
        &self.server.state.file_permits
    }

    /// Seeds an asset row of `content_type` in the world and writes `files` (suffix, bytes) beside
    /// its canonical path. Returns the id.
    async fn seed(&self, content_type: &str, files: &[(&str, &[u8])]) -> Uuid {
        let state = &self.server.state;
        let id = Uuid::new_v4();
        let world = self.server.world;
        state
            .repo
            .insert_asset(&Asset {
                id,
                world_id: world,
                storage_key: format!("{world}/{id}"),
                original_name: "seeded.bin".into(),
                content_type: content_type.into(),
                byte_size: 0,
                created_by: None,
                created_at: 0,
                version: 1,
                folder_id: None,
                tags: vec![],
                derived_tags: vec![],
                meta: AssetMeta::unprocessed(content_type, 0),
            })
            .await
            .unwrap();
        let folder = self.dir.path().join(world.to_string());
        std::fs::create_dir_all(&folder).unwrap();
        for (suffix, bytes) in files {
            std::fs::write(folder.join(format!("{id}{suffix}")), bytes).unwrap();
        }
        id
    }

    /// Seeds a media asset whose Opus derivative is `MEDIA_LEN` patterned bytes. Returns its id and
    /// the derivative's path.
    async fn seed_media(&self) -> (Uuid, PathBuf) {
        let id = self
            .seed("audio/wav", &[(".opus.ogg", &pattern(MEDIA_LEN))])
            .await;
        let path = self
            .dir
            .path()
            .join(self.server.world.to_string())
            .join(format!("{id}.opus.ogg"));
        (id, path)
    }

    /// Shuts the loop down and returns its drain report, asserting no connection needed a kill.
    async fn finish(self) -> DrainReport {
        let drain = self.server.finish().await;
        assert_eq!(drain.dropped, 0, "{drain:?}");
        assert_eq!(drain.aborted, 0, "{drain:?}");
        assert_eq!(drain.panicked, 0, "{drain:?}");
        drain
    }
}

/// The cut counts one stalled response produces on `protocol`: a stream cut on HTTP/2, a
/// connection cut on HTTP/1.
fn one_cut(protocol: Protocol) -> StallCuts {
    match protocol {
        Protocol::H1 => StallCuts {
            stream: 0,
            connection: 1,
            ended_unflushed: 0,
            tracker_overflow: 0,
        },
        Protocol::H2 => StallCuts {
            stream: 1,
            connection: 0,
            ended_unflushed: 0,
            tracker_overflow: 0,
        },
    }
}

/// Runs `body` once per protocol.
macro_rules! per_protocol {
    ($name:ident, $protocol:ident, $body:expr) => {
        #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
        async fn $name() {
            let run: fn(Protocol) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()>>> =
                |$protocol| Box::pin($body);
            run(Protocol::H1).await;
            run(Protocol::H2).await;
        }
    };
}

per_protocol!(
    a_stalled_response_is_cut_and_releases_its_permit_and_its_file_handle,
    protocol,
    async move {
        let mut media = Media::start(protocol, 4).await;
        let (id, path) = media.seed_media().await;
        let started = Instant::now();
        let reply = media
            .get(&format!("/api/assets/{id}?variant=opus"), &[])
            .await;
        assert_eq!(reply.status(), 200, "{protocol:?}");
        assert_eq!(
            reply.header("content-length"),
            Some(&*MEDIA_LEN.to_string())
        );
        assert!(
            wait_for(Duration::from_secs(5), || media.permits().held() == 1).await,
            "{protocol:?}: the response holds no permit while it streams"
        );
        assert!(file_is_open(&path), "{protocol:?}: the file is not open");

        // Not cut before the window: still held half a window in.
        tokio::time::sleep(WINDOW / 2).await;
        assert_eq!(
            media.permits().held(),
            1,
            "{protocol:?}: cut inside the window"
        );

        // The peer reads nothing; the response is cut and both resources come back.
        assert!(
            wait_for(WINDOW + TICK + SLACK, || media.permits().held() == 0).await,
            "{protocol:?}: the cut never released the permit"
        );
        assert!(
            started.elapsed() >= WINDOW,
            "{protocol:?}: released {:?} after the request, before the window",
            started.elapsed()
        );
        assert!(
            wait_for(Duration::from_secs(5), || !file_is_open(&path)).await,
            "{protocol:?}: the cut left the file open"
        );
        drop(reply);
        assert_eq!(media.finish().await.stall_cuts, one_cut(protocol));
    }
);

per_protocol!(
    a_seek_past_the_buffer_lands_on_the_exact_bytes,
    protocol,
    async move {
        let mut media = Media::start(protocol, 8).await;
        let (id, _) = media.seed_media().await;
        let url = format!("/api/assets/{id}?variant=opus");
        let file = pattern(MEDIA_LEN);

        let mut first = media.get(&url, &[("range", "bytes=0-".into())]).await;
        assert_eq!(first.status(), 206, "{protocol:?}");
        assert_eq!(first.read_all().await.unwrap(), &file[..SLICE]);

        let seek = MEDIA_LEN - SLICE;
        let mut tail = media
            .get(&url, &[("range", format!("bytes={seek}-"))])
            .await;
        assert_eq!(tail.status(), 206, "{protocol:?}");
        assert_eq!(
            tail.header("content-range"),
            Some(&*format!("bytes {seek}-{}/{MEDIA_LEN}", MEDIA_LEN - 1))
        );
        assert_eq!(tail.read_all().await.unwrap(), &file[seek..]);
        media.finish().await;
    }
);

per_protocol!(
    continuous_playback_never_leaves_a_response_in_flight_past_the_window,
    protocol,
    async move {
        let mut media = Media::start(protocol, 8).await;
        let (id, _) = media.seed_media().await;
        let url = format!("/api/assets/{id}?variant=opus");
        let file = pattern(MEDIA_LEN);
        // A media element reads what it asked for, then pauses far longer than the window before
        // asking again from where it stopped.
        let mut played = Vec::new();
        while played.len() < MEDIA_LEN {
            let mut reply = media
                .get(&url, &[("range", format!("bytes={}-", played.len()))])
                .await;
            assert_eq!(reply.status(), 206, "{protocol:?} at {}", played.len());
            let slice = reply.read_at_least(SLICE).await.unwrap();
            played.extend_from_slice(&slice);
            // The slice ended with the response: nothing is in flight during the pause.
            assert_eq!(
                media.permits().held(),
                0,
                "{protocol:?}: a response is still open after {} bytes",
                played.len()
            );
            tokio::time::sleep(WINDOW * 2).await;
        }
        assert_eq!(played, file, "{protocol:?}");
        let drain = media.finish().await;
        assert_eq!(drain.stall_cuts, StallCuts::default(), "{protocol:?}");
    }
);

per_protocol!(
    a_cut_response_is_recovered_through_a_range_request,
    protocol,
    async move {
        let mut media = Media::start(protocol, 8).await;
        let (id, _) = media.seed_media().await;
        let url = format!("/api/assets/{id}?variant=opus");
        let file = pattern(MEDIA_LEN);

        // A full response the peer stops reading half way: it is cut after the window.
        let mut reply = media.get(&url, &[]).await;
        assert_eq!(reply.status(), 200, "{protocol:?}");
        let mut received = reply.read_at_least(SLICE / 2).await.unwrap();
        tokio::time::sleep(WINDOW + TICK + SLACK).await;
        // Whatever the kernel and h2 had buffered still arrives; the rest never does.
        let cut = loop {
            match reply.read_some().await {
                Ok(Some(chunk)) => received.extend_from_slice(&chunk),
                Ok(None) => break false,
                Err(Cut) => break true,
            }
        };
        assert!(cut, "{protocol:?}: the stalled response was not cut");
        assert!(received.len() < MEDIA_LEN, "{protocol:?}");
        assert_eq!(received, &file[..received.len()], "{protocol:?}");

        // Resuming from the first byte not received completes the file byte-exact.
        let mut resumed = media
            .get(&url, &[("range", format!("bytes={}-", received.len()))])
            .await;
        assert_eq!(resumed.status(), 206, "{protocol:?}");
        let mut rest = resumed.read_all().await.unwrap();
        while received.len() + rest.len() < MEDIA_LEN {
            let mut next = media
                .get(
                    &url,
                    &[("range", format!("bytes={}-", received.len() + rest.len()))],
                )
                .await;
            rest.extend_from_slice(&next.read_all().await.unwrap());
        }
        received.extend_from_slice(&rest);
        assert_eq!(received, file, "{protocol:?}");
        let drain = media.finish().await;
        assert_eq!(drain.stall_cuts, one_cut(protocol), "{protocol:?}");
    }
);

per_protocol!(
    a_large_compressed_full_response_is_cut_when_its_peer_stops_reading,
    protocol,
    async move {
        let mut media = Media::start(protocol, 4).await;
        let text = incompressible_text(12 * 1024 * 1024);
        let id = media.seed("text/plain", &[("", &text)]).await;
        let reply = media
            .get(
                &format!("/api/assets/{id}"),
                &[("accept-encoding", "gzip".into())],
            )
            .await;
        assert_eq!(reply.status(), 200, "{protocol:?}");
        assert_eq!(
            reply.header("content-encoding"),
            Some("gzip"),
            "{protocol:?}"
        );
        assert!(reply.header("accept-ranges").is_none(), "{protocol:?}");
        assert!(
            wait_for(Duration::from_secs(5), || media.permits().held() == 1).await,
            "{protocol:?}: the response holds no permit while it streams"
        );
        assert!(
            wait_for(WINDOW + TICK + SLACK, || media.permits().held() == 0).await,
            "{protocol:?}: the stalled compressed response was never cut"
        );
        drop(reply);
        assert_eq!(media.finish().await.stall_cuts, one_cut(protocol));
    }
);

/// How many `EGRESS_CHUNK_BYTES` per window the progressing reader takes; the same margin the
/// connection-unit controls use, so the loopback's lumpy acceptance does not read as a stall.
const PROGRESS_CHUNKS_PER_WINDOW: usize = 64;

per_protocol!(
    a_reader_that_keeps_reading_a_compressed_response_is_not_cut_across_three_windows,
    protocol,
    async move {
        let mut media = Media::start(protocol, 4).await;
        let text = incompressible_text(12 * 1024 * 1024);
        let id = media.seed("text/plain", &[("", &text)]).await;
        let mut reply = media
            .get(
                &format!("/api/assets/{id}"),
                &[("accept-encoding", "gzip".into())],
            )
            .await;
        assert_eq!(reply.status(), 200, "{protocol:?}");
        let per_window =
            PROGRESS_CHUNKS_PER_WINDOW * shadowcat::http::conn_serve::EGRESS_CHUNK_BYTES;
        let mut read = 0usize;
        let start = Instant::now();
        while start.elapsed() < WINDOW * 3 {
            let chunk = reply
                .read_some()
                .await
                .expect("a reader above the floor was cut")
                .expect("the body is far longer than three windows of reading");
            read += chunk.len();
            // Paced to `per_window` bytes per window.
            let due = start + Duration::from_secs_f64(read as f64 / per_window as f64);
            tokio::time::sleep_until(due).await;
        }
        assert!(
            read > 2 * per_window,
            "{protocol:?}: read only {read} bytes"
        );
        drop(reply);
        let drain = media.finish().await;
        assert_eq!(drain.stall_cuts, StallCuts::default(), "{protocol:?}");
    }
);

per_protocol!(
    a_drain_with_a_response_open_ends_within_the_drain_bound,
    protocol,
    async move {
        let mut media = Media::start(protocol, 4).await;
        let (id, path) = media.seed_media().await;
        let reply = media
            .get(&format!("/api/assets/{id}?variant=opus"), &[])
            .await;
        assert_eq!(reply.status(), 200, "{protocol:?}");
        assert!(
            wait_for(Duration::from_secs(5), || media.permits().held() == 1).await,
            "{protocol:?}"
        );
        let permits = media.server.state.file_permits.clone();
        let started = Instant::now();
        let _drain = media.server.finish().await;
        let took = started.elapsed();
        // The deadline closes the connection by failing its IO; the loop then waits at most the
        // kill grace and the abort wait for the connection task to end.
        assert!(
            took < DRAIN + ABORT_WAIT + SLACK,
            "{protocol:?}: the drain took {took:?}"
        );
        drop(reply);
        assert_eq!(
            permits.held(),
            0,
            "{protocol:?}: the drain left a permit held"
        );
        assert!(
            !file_is_open(&path),
            "{protocol:?}: the drain left the file open"
        );
        drop(media.dir);
    }
);

per_protocol!(
    requests_past_the_permit_pool_are_refused_with_retry_after_while_the_first_still_stream,
    protocol,
    async move {
        let mut media = Media::start(protocol, 2).await;
        let (id, _) = media.seed_media().await;
        let url = format!("/api/assets/{id}?variant=opus");
        let first = media.get(&url, &[]).await;
        let second = media.get(&url, &[]).await;
        assert_eq!(
            (first.status(), second.status()),
            (200, 200),
            "{protocol:?}"
        );
        assert!(
            wait_for(Duration::from_secs(5), || media.permits().held() == 2).await,
            "{protocol:?}"
        );

        // The pool is empty: the third waits its bound and is refused before any header.
        let asked = Instant::now();
        let refused = media.get(&url, &[]).await;
        assert_eq!(refused.status(), 503, "{protocol:?}");
        assert_eq!(refused.header("retry-after"), Some("1"), "{protocol:?}");
        assert!(
            asked.elapsed() >= PERMIT_WAIT,
            "{protocol:?}: refused after {:?}, before the permit wait",
            asked.elapsed()
        );
        // The first two were not disturbed by the refusal.
        assert_eq!(media.permits().held(), 2, "{protocol:?}");
        assert!(media.permits().peak() >= 2);

        // After they are cut the pool serves again.
        assert!(
            wait_for(WINDOW + TICK + SLACK, || media.permits().held() == 0).await,
            "{protocol:?}"
        );
        let mut again = media.get(&url, &[("range", "bytes=0-9".into())]).await;
        assert_eq!(again.status(), 206, "{protocol:?}");
        assert_eq!(again.read_all().await.unwrap(), pattern(10), "{protocol:?}");
        drop((first, second, refused));
        media.finish().await;
    }
);

per_protocol!(
    a_client_that_goes_away_returns_the_permit_long_before_the_stall_window,
    protocol,
    async move {
        let mut media = Media::start(protocol, 1).await;
        let (id, path) = media.seed_media().await;
        let reply = media
            .get(&format!("/api/assets/{id}?variant=opus"), &[])
            .await;
        assert_eq!(reply.status(), 200, "{protocol:?}");
        assert!(
            wait_for(Duration::from_secs(5), || media.permits().held() == 1).await,
            "{protocol:?}"
        );
        // The peer resets (HTTP/2) or closes (HTTP/1) with the body unread.
        let gone = Instant::now();
        drop(reply);
        assert!(
            wait_for(WINDOW / 2, || media.permits().held() == 0).await,
            "{protocol:?}: the permit was still held {:?} after the client went away",
            gone.elapsed()
        );
        assert!(
            wait_for(Duration::from_secs(5), || !file_is_open(&path)).await,
            "{protocol:?}: the file stayed open after the client went away"
        );
        let drain = media.finish().await;
        assert_eq!(drain.stall_cuts, StallCuts::default(), "{protocol:?}");
    }
);

/// A response whose file has been read to its end holds no permit even though its client has read
/// nothing and HTTP/2 still buffers the tail: a second request is served with a one-permit pool.
/// This pins the observable only; the body is polled to its end at once either way, so the
/// release at the last file byte (before that final poll) is pinned by
/// `http::assets::file_response`'s unit test, not here.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn http2_returns_the_permit_at_the_last_file_byte_while_the_client_has_read_nothing() {
    let mut media = Media::start(Protocol::H2, 1).await;
    // Small enough for the stream's initial window and the send buffer to take whole.
    let small = media
        .seed("audio/wav", &[(".opus.ogg", &pattern(20 * 1024))])
        .await;
    let url = format!("/api/assets/{small}?variant=opus");
    let first = media.get(&url, &[]).await;
    assert_eq!(first.status(), 200);
    assert!(
        wait_for(Duration::from_secs(5), || media.permits().held() == 0).await,
        "the permit was still held with every file byte already read"
    );
    // The first body is still unread; the pool of one serves another request.
    let mut second = media.get(&url, &[("range", "bytes=0-9".into())]).await;
    assert_eq!(second.status(), 206);
    assert_eq!(second.read_all().await.unwrap(), pattern(10));
    let mut first = first;
    assert_eq!(first.read_all().await.unwrap(), pattern(20 * 1024));
    media.finish().await;
}

/// A world with three large asset rows, so its export is far larger than the buffers between the
/// server and the client, and an administrator who can request it.
async fn export_fixture(permits: usize) -> (Media, String) {
    let mut media = Media::start(Protocol::H1, permits).await;
    for _ in 0..3 {
        media
            .seed("image/webp", &[("", &pattern(12 * 1024 * 1024))])
            .await;
    }
    let hash = hash_password("pw").unwrap();
    media
        .server
        .state
        .repo
        .create_user("admin", Some(&hash), ServerRole::Admin, 0)
        .await
        .unwrap();
    let cookie = media.login("admin").await;
    (media, cookie)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_cut_export_releases_the_barrier_and_the_permit() {
    let (media, cookie) = export_fixture(2).await;
    let world = media.server.world;
    let addr = media.server.addr;
    let state = media.server.state.clone();
    let reply = h1_request(
        addr,
        "POST",
        &format!("/api/worlds/{world}/export"),
        &[("cookie", cookie)],
        &[],
    )
    .await;
    assert_eq!(reply.status(), 200);
    assert!(
        wait_for(Duration::from_secs(5), || state.file_permits.held() == 1).await,
        "the export holds no permit while it streams"
    );
    assert!(
        state.write_barrier.try_write().is_err(),
        "the export does not hold the write barrier while it streams"
    );

    // The client reads nothing: the connection is cut, the dropped receiver ends the producing
    // thread, and the task that held both resources returns them.
    assert!(
        wait_for(WINDOW + TICK + SLACK + Duration::from_secs(5), || {
            state.file_permits.held() == 0
        })
        .await,
        "the cut export never released its permit"
    );
    assert!(
        wait_for(Duration::from_secs(5), || state
            .write_barrier
            .try_write()
            .is_ok())
        .await,
        "the cut export never released the write barrier"
    );
    drop(reply);
    assert_eq!(media.finish().await.stall_cuts, one_cut(Protocol::H1));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_export_with_every_permit_held_is_refused_before_its_headers() {
    let (mut media, cookie) = export_fixture(1).await;
    let (id, _) = media.seed_media().await;
    let holder = media
        .get(&format!("/api/assets/{id}?variant=opus"), &[])
        .await;
    assert_eq!(holder.status(), 200);
    assert!(wait_for(Duration::from_secs(5), || media.permits().held() == 1).await);
    let world = media.server.world;
    let refused = h1_request(
        media.server.addr,
        "POST",
        &format!("/api/worlds/{world}/export"),
        &[("cookie", cookie)],
        &[],
    )
    .await;
    assert_eq!(refused.status(), 503);
    assert_eq!(refused.header("retry-after"), Some("1"));
    // The refusal released the write barrier it took first.
    assert!(media.server.state.write_barrier.try_write().is_ok());
    drop((holder, refused));
    media.finish().await;
}
