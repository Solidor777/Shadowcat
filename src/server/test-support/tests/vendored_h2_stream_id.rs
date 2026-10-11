//! The vendored `h2` hands the stream id to the request as the public `h2::StreamId` extension.
//!
//! It does so on the request an `h2` server accepts and on the request hyper passes to a service.
//!
//! Both tests run over `tokio::io::duplex` (no sockets) and in every profile.

use bytes::Bytes;
use http::{Method, Request, Response};

/// Opens `count` GET requests on one h2 client connection and awaits each response.
async fn client(io: tokio::io::DuplexStream, count: usize) {
    let (mut send_request, connection) = h2::client::handshake(io).await.unwrap();
    let driver = tokio::spawn(connection);
    for _ in 0..count {
        let request = Request::builder()
            .method(Method::GET)
            .uri("https://example.com/")
            .body(())
            .unwrap();
        let (response, _) = send_request.send_request(request, true).unwrap();
        response.await.unwrap();
    }
    drop(send_request);
    driver.abort();
}

/// An `h2` server's accepted requests carry the ids of their streams, in the odd client-initiated
/// sequence.
#[tokio::test]
async fn an_accepted_request_carries_its_stream_id() {
    let (server_io, client_io) = tokio::io::duplex(64 * 1024);
    let server = tokio::spawn(async move {
        let mut connection = h2::server::handshake(server_io).await.unwrap();
        let mut ids = Vec::new();
        // Runs until the client ends the connection, so every response is flushed first.
        while let Some(Ok((request, mut respond))) = connection.accept().await {
            let id = request.extensions().get::<h2::StreamId>().copied();
            ids.push(id.map(u32::from));
            respond
                .send_response(Response::builder().status(200).body(()).unwrap(), true)
                .unwrap();
        }
        ids
    });
    client(client_io, 2).await;
    assert_eq!(server.await.unwrap(), vec![Some(1), Some(3)]);
}

/// A hyper service over the same transport sees the extension on the request it is called with.
#[tokio::test]
async fn a_hyper_service_request_carries_its_stream_id() {
    let (server_io, client_io) = tokio::io::duplex(64 * 1024);
    let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let seen_in_service = seen.clone();
    let service = hyper::service::service_fn(move |request: Request<hyper::body::Incoming>| {
        let id = request.extensions().get::<h2::StreamId>().copied();
        seen_in_service.lock().unwrap().push(id.map(u32::from));
        async {
            Ok::<_, std::convert::Infallible>(Response::new(http_body_util::Empty::<Bytes>::new()))
        }
    });
    let server = tokio::spawn(async move {
        let connection =
            hyper::server::conn::http2::Builder::new(hyper_util::rt::TokioExecutor::new())
                .serve_connection(hyper_util::rt::TokioIo::new(server_io), service);
        let _ = connection.await;
    });
    client(client_io, 2).await;
    let _ = server.await;
    assert_eq!(*seen.lock().unwrap(), vec![Some(1), Some(3)]);
}
