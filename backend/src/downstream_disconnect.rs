use std::{
    convert::Infallible,
    future::{Future, Ready, ready},
    io::{self, IoSlice},
    net::SocketAddr,
    pin::Pin,
    task::{Context, Poll},
};

use axum::{
    Extension, Router,
    extract::ConnectInfo,
    http::Request,
    serve::{IncomingStream, Listener},
};
use futures::Stream;
use tokio::{
    io::{AsyncRead, AsyncWrite, ReadBuf},
    net::{TcpListener, TcpStream},
};
use tokio_util::sync::CancellationToken;
use tower::Service;

/// Connection-level cancellation propagated from the server transport.
///
/// Request handlers create child tokens so dropping one response body never
/// cancels another request on the same keep-alive connection.
#[derive(Clone)]
pub(crate) struct ClientConnectionCancellation(CancellationToken);

impl ClientConnectionCancellation {
    fn new(token: CancellationToken) -> Self {
        Self(token)
    }

    fn child_token(&self) -> CancellationToken {
        self.0.child_token()
    }
}

/// Return a request-scoped token cancelled when its client connection closes.
/// Requests constructed without the production listener receive an independent
/// token, which keeps direct handler tests and in-process callers usable.
pub(crate) fn request_cancellation<B>(request: &Request<B>) -> CancellationToken {
    request
        .extensions()
        .get::<ClientConnectionCancellation>()
        .map(ClientConnectionCancellation::child_token)
        .unwrap_or_default()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ClientDisconnected;

/// Poll work only while its downstream request remains observable.
///
/// The work branch is biased so a completed upstream result wins a simultaneous
/// disconnect race. Dropping the losing future performs cancellation through
/// the upstream client's normal RAII semantics.
pub(crate) async fn until_client_disconnect<F>(
    cancellation: &CancellationToken,
    work: F,
) -> Result<F::Output, ClientDisconnected>
where
    F: Future,
{
    tokio::select! {
        biased;
        output = work => Ok(output),
        () = cancellation.cancelled() => Err(ClientDisconnected),
    }
}

/// A stream that cancels its request token as soon as the downstream response
/// body is dropped. The inner stream is dropped at the same point, which owns
/// and cancels ordinary reqwest response bodies without a detached task.
pub(crate) struct CancelOnDropStream<S> {
    inner: Pin<Box<S>>,
    cancellation: CancellationToken,
}

impl<S> CancelOnDropStream<S> {
    pub(crate) fn new(inner: S, cancellation: CancellationToken) -> Self {
        Self {
            inner: Box::pin(inner),
            cancellation,
        }
    }
}

impl<S> Stream for CancelOnDropStream<S>
where
    S: Stream,
{
    type Item = S::Item;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.get_mut().inner.as_mut().poll_next(cx)
    }
}

impl<S> Drop for CancelOnDropStream<S> {
    fn drop(&mut self) {
        self.cancellation.cancel();
    }
}

pub(crate) struct DisconnectAwareListener {
    inner: TcpListener,
}

impl DisconnectAwareListener {
    pub(crate) fn new(inner: TcpListener) -> Self {
        Self { inner }
    }
}

impl Listener for DisconnectAwareListener {
    type Io = DisconnectAwareIo;
    type Addr = SocketAddr;

    async fn accept(&mut self) -> (Self::Io, Self::Addr) {
        let (stream, peer) = <TcpListener as Listener>::accept(&mut self.inner).await;
        (DisconnectAwareIo::new(stream), peer)
    }

    fn local_addr(&self) -> io::Result<Self::Addr> {
        self.inner.local_addr()
    }
}

pub(crate) struct DisconnectAwareIo {
    inner: TcpStream,
    cancellation: CancellationToken,
}

impl DisconnectAwareIo {
    fn new(inner: TcpStream) -> Self {
        Self {
            inner,
            cancellation: CancellationToken::new(),
        }
    }
}

impl Drop for DisconnectAwareIo {
    fn drop(&mut self) {
        self.cancellation.cancel();
    }
}

impl AsyncRead for DisconnectAwareIo {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        let filled_before = buf.filled().len();
        let remaining_before = buf.remaining();
        match Pin::new(&mut this.inner).poll_read(cx, buf) {
            Poll::Ready(Ok(())) if remaining_before > 0 && buf.filled().len() == filled_before => {
                this.cancellation.cancel();
                Poll::Ready(Ok(()))
            }
            Poll::Ready(Err(error)) => {
                this.cancellation.cancel();
                Poll::Ready(Err(error))
            }
            result => result,
        }
    }
}

impl AsyncWrite for DisconnectAwareIo {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        match Pin::new(&mut this.inner).poll_write(cx, buf) {
            Poll::Ready(Ok(0)) if !buf.is_empty() => {
                this.cancellation.cancel();
                Poll::Ready(Ok(0))
            }
            Poll::Ready(Err(error)) => {
                this.cancellation.cancel();
                Poll::Ready(Err(error))
            }
            result => result,
        }
    }

    fn poll_write_vectored(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        let has_bytes = bufs.iter().any(|buf| !buf.is_empty());
        match Pin::new(&mut this.inner).poll_write_vectored(cx, bufs) {
            Poll::Ready(Ok(0)) if has_bytes => {
                this.cancellation.cancel();
                Poll::Ready(Ok(0))
            }
            Poll::Ready(Err(error)) => {
                this.cancellation.cancel();
                Poll::Ready(Err(error))
            }
            result => result,
        }
    }

    fn is_write_vectored(&self) -> bool {
        self.inner.is_write_vectored()
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        match Pin::new(&mut this.inner).poll_flush(cx) {
            Poll::Ready(Err(error)) => {
                this.cancellation.cancel();
                Poll::Ready(Err(error))
            }
            result => result,
        }
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        match Pin::new(&mut this.inner).poll_shutdown(cx) {
            Poll::Ready(result) => {
                this.cancellation.cancel();
                Poll::Ready(result)
            }
            Poll::Pending => Poll::Pending,
        }
    }
}

#[derive(Clone)]
pub(crate) struct DisconnectAwareMakeService {
    app: Router,
}

impl DisconnectAwareMakeService {
    pub(crate) fn new(app: Router) -> Self {
        Self { app }
    }
}

impl Service<IncomingStream<'_, DisconnectAwareListener>> for DisconnectAwareMakeService {
    type Response = Router;
    type Error = Infallible;
    type Future = Ready<Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, stream: IncomingStream<'_, DisconnectAwareListener>) -> Self::Future {
        let peer = *stream.remote_addr();
        let cancellation = stream.io().cancellation.clone();
        let service = Router::layer(self.app.clone(), Extension(ConnectInfo(peer)));
        let service = Router::layer(
            service,
            Extension(ClientConnectionCancellation::new(cancellation)),
        );
        ready(Ok(service))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::Body,
        extract::{Path, State},
        http::{Response, StatusCode},
        response::IntoResponse,
        routing::get,
    };
    use bytes::Bytes;
    use futures::StreamExt;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        sync::oneshot,
        task::JoinHandle,
        time::{Duration, timeout},
    };

    #[derive(Clone)]
    struct ProxyTestState {
        client: reqwest::Client,
        upstream_base_url: String,
    }

    async fn lifecycle_proxy(
        State(state): State<ProxyTestState>,
        Path(mode): Path<String>,
        request: Request<Body>,
    ) -> Response<Body> {
        assert!(
            request
                .extensions()
                .get::<ConnectInfo<SocketAddr>>()
                .is_some(),
            "disconnect-aware serving must preserve peer connect info"
        );
        let cancellation = request_cancellation(&request);
        let upstream_url = format!("{}/{}", state.upstream_base_url, mode);
        let upstream =
            match until_client_disconnect(&cancellation, state.client.get(upstream_url).send())
                .await
            {
                Ok(Ok(response)) => response,
                Ok(Err(_)) => return StatusCode::BAD_GATEWAY.into_response(),
                Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
            };

        if mode == "detached" {
            let mut upstream_stream = upstream.bytes_stream();
            let task_cancellation = cancellation.clone();
            let (tx, rx) = tokio::sync::mpsc::channel::<Result<Bytes, io::Error>>(4);
            tokio::spawn(async move {
                loop {
                    match until_client_disconnect(&task_cancellation, upstream_stream.next()).await
                    {
                        Err(_) => {
                            drop(upstream_stream);
                            return;
                        }
                        Ok(Some(Ok(bytes))) => {
                            if tx.send(Ok(bytes)).await.is_err() {
                                drop(upstream_stream);
                                return;
                            }
                        }
                        Ok(Some(Err(error))) => {
                            let _ = tx.send(Err(io::Error::other(error))).await;
                            return;
                        }
                        Ok(None) => return,
                    }
                }
            });
            let stream = CancelOnDropStream::new(
                tokio_stream::wrappers::ReceiverStream::new(rx),
                cancellation,
            );
            Response::builder()
                .status(StatusCode::OK)
                .header("content-type", "text/event-stream")
                .body(Body::from_stream(stream))
                .expect("build detached streaming proxy response")
        } else if mode == "stream" {
            let stream = CancelOnDropStream::new(upstream.bytes_stream(), cancellation);
            Response::builder()
                .status(StatusCode::OK)
                .header("content-type", "application/octet-stream")
                .body(Body::from_stream(stream))
                .expect("build streaming proxy response")
        } else {
            match until_client_disconnect(&cancellation, upstream.bytes()).await {
                Ok(Ok(bytes)) => Response::new(Body::from(bytes)),
                Ok(Err(_)) => StatusCode::BAD_GATEWAY.into_response(),
                Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
            }
        }
    }

    async fn start_proxy(upstream_base_url: String) -> (SocketAddr, JoinHandle<()>) {
        let app = Router::new()
            .route("/{mode}", get(lifecycle_proxy))
            .with_state(ProxyTestState {
                client: reqwest::Client::new(),
                upstream_base_url,
            });
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind lifecycle proxy");
        let addr = listener.local_addr().expect("lifecycle proxy address");
        let server = tokio::spawn(async move {
            axum::serve(
                DisconnectAwareListener::new(listener),
                DisconnectAwareMakeService::new(app),
            )
            .await
            .expect("serve lifecycle proxy");
        });
        (addr, server)
    }

    async fn read_request_headers(stream: &mut TcpStream) {
        let mut request = Vec::new();
        let mut chunk = [0_u8; 1024];
        while !request.windows(4).any(|window| window == b"\r\n\r\n") {
            let read = stream
                .read(&mut chunk)
                .await
                .expect("read upstream request");
            assert!(read > 0, "proxy closed upstream before sending headers");
            request.extend_from_slice(&chunk[..read]);
        }
    }

    #[derive(Clone, Copy)]
    enum CancellationPhase {
        BeforeHeaders,
        BufferedBody,
        DetachedStream,
    }

    async fn start_cancellation_upstream(
        phase: CancellationPhase,
    ) -> (
        String,
        oneshot::Receiver<()>,
        oneshot::Receiver<()>,
        JoinHandle<()>,
    ) {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind cancellation upstream");
        let addr = listener
            .local_addr()
            .expect("cancellation upstream address");
        let (ready_tx, ready_rx) = oneshot::channel();
        let (cancelled_tx, cancelled_rx) = oneshot::channel();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("accept proxy connection");
            read_request_headers(&mut stream).await;

            match phase {
                CancellationPhase::BeforeHeaders => {}
                CancellationPhase::BufferedBody => {
                    stream
                        .write_all(
                            b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 1048576\r\n\r\npartial",
                        )
                        .await
                        .expect("write partial buffered response");
                    stream
                        .flush()
                        .await
                        .expect("flush partial buffered response");
                }
                CancellationPhase::DetachedStream => {
                    stream
                        .write_all(
                            b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\nf\r\ndata: partial\n\n\r\n",
                        )
                        .await
                        .expect("write partial streaming response");
                    stream
                        .flush()
                        .await
                        .expect("flush partial streaming response");
                }
            }
            ready_tx.send(()).expect("signal upstream ready");

            let mut byte = [0_u8; 1];
            loop {
                match stream.read(&mut byte).await {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {}
                }
            }
            let _ = cancelled_tx.send(());
        });
        (format!("http://{addr}"), ready_rx, cancelled_rx, server)
    }

    async fn open_downstream_request(proxy_addr: SocketAddr, path: &str) -> TcpStream {
        let mut stream = TcpStream::connect(proxy_addr)
            .await
            .expect("connect downstream client");
        stream
            .write_all(format!("GET /{path} HTTP/1.1\r\nHost: {proxy_addr}\r\n\r\n").as_bytes())
            .await
            .expect("write downstream request");
        stream.flush().await.expect("flush downstream request");
        stream
    }

    #[tokio::test]
    async fn disconnect_cancels_upstream_before_response_headers() {
        let (upstream_url, upstream_ready, upstream_cancelled, upstream_server) =
            start_cancellation_upstream(CancellationPhase::BeforeHeaders).await;
        let (proxy_addr, proxy_server) = start_proxy(upstream_url).await;
        let downstream = open_downstream_request(proxy_addr, "buffer").await;

        timeout(Duration::from_secs(1), upstream_ready)
            .await
            .expect("upstream request did not start")
            .expect("upstream readiness sender dropped");
        drop(downstream);
        timeout(Duration::from_secs(1), upstream_cancelled)
            .await
            .expect("upstream request was not cancelled promptly")
            .expect("upstream cancellation sender dropped");

        proxy_server.abort();
        upstream_server.await.expect("cancellation upstream task");
    }

    #[tokio::test]
    async fn disconnect_cancels_upstream_while_buffering_response_body() {
        let (upstream_url, upstream_ready, upstream_cancelled, upstream_server) =
            start_cancellation_upstream(CancellationPhase::BufferedBody).await;
        let (proxy_addr, proxy_server) = start_proxy(upstream_url).await;
        let downstream = open_downstream_request(proxy_addr, "buffer").await;

        timeout(Duration::from_secs(1), upstream_ready)
            .await
            .expect("upstream body did not start")
            .expect("upstream readiness sender dropped");
        drop(downstream);
        timeout(Duration::from_secs(1), upstream_cancelled)
            .await
            .expect("buffered upstream body was not cancelled promptly")
            .expect("upstream cancellation sender dropped");

        proxy_server.abort();
        upstream_server.await.expect("cancellation upstream task");
    }

    #[tokio::test]
    async fn response_body_drop_cancels_a_silent_detached_upstream_stream() {
        let (upstream_url, upstream_ready, upstream_cancelled, upstream_server) =
            start_cancellation_upstream(CancellationPhase::DetachedStream).await;
        let (proxy_addr, proxy_server) = start_proxy(upstream_url).await;
        let mut downstream = open_downstream_request(proxy_addr, "detached").await;

        timeout(Duration::from_secs(1), upstream_ready)
            .await
            .expect("upstream stream did not start")
            .expect("upstream readiness sender dropped");
        timeout(Duration::from_secs(1), async {
            let mut response = Vec::new();
            let mut chunk = [0_u8; 1024];
            while !response
                .windows(b"data: partial".len())
                .any(|window| window == b"data: partial")
            {
                let read = downstream
                    .read(&mut chunk)
                    .await
                    .expect("read downstream stream");
                assert!(read > 0, "downstream stream ended before its first event");
                response.extend_from_slice(&chunk[..read]);
            }
        })
        .await
        .expect("proxy did not forward the first streaming event");
        drop(downstream);
        timeout(Duration::from_secs(1), upstream_cancelled)
            .await
            .expect("detached upstream stream was not cancelled promptly")
            .expect("upstream cancellation sender dropped");

        proxy_server.abort();
        upstream_server.await.expect("cancellation upstream task");
    }

    async fn normal_buffered() -> Response<Body> {
        Response::builder()
            .status(StatusCode::OK)
            .header("content-type", "application/json")
            .header("content-length", "17")
            .body(Body::from("buffered-response"))
            .expect("build buffered upstream response")
    }

    async fn normal_streaming() -> Response<Body> {
        let chunks = futures::stream::iter([
            Ok::<_, Infallible>(Bytes::from_static(b"stream-")),
            Ok::<_, Infallible>(Bytes::from_static(b"response")),
        ]);
        Response::builder()
            .status(StatusCode::OK)
            .header("content-type", "application/octet-stream")
            .body(Body::from_stream(chunks))
            .expect("build streaming upstream response")
    }

    #[tokio::test]
    async fn normal_buffered_and_streaming_responses_complete() {
        let upstream_app = Router::new()
            .route("/buffer", get(normal_buffered))
            .route("/stream", get(normal_streaming));
        let upstream_listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind normal upstream");
        let upstream_addr = upstream_listener
            .local_addr()
            .expect("normal upstream address");
        let upstream_server = tokio::spawn(async move {
            axum::serve(upstream_listener, upstream_app)
                .await
                .expect("serve normal upstream");
        });
        let (proxy_addr, proxy_server) = start_proxy(format!("http://{upstream_addr}")).await;
        let client = reqwest::Client::new();

        let buffered = client
            .get(format!("http://{proxy_addr}/buffer"))
            .send()
            .await
            .expect("buffered proxy request")
            .bytes()
            .await
            .expect("buffered proxy body");
        assert_eq!(buffered, Bytes::from_static(b"buffered-response"));

        let response = client
            .get(format!("http://{proxy_addr}/stream"))
            .send()
            .await
            .expect("streaming proxy request");
        let chunks = response
            .bytes_stream()
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .collect::<Result<Vec<_>, _>>()
            .expect("streaming proxy body");
        assert_eq!(chunks.concat(), b"stream-response");

        let buffered_again = client
            .get(format!("http://{proxy_addr}/buffer"))
            .send()
            .await
            .expect("second buffered proxy request")
            .bytes()
            .await
            .expect("second buffered proxy body");
        assert_eq!(buffered_again, Bytes::from_static(b"buffered-response"));

        proxy_server.abort();
        upstream_server.abort();
    }

    #[tokio::test]
    async fn completed_work_wins_a_simultaneous_disconnect_race() {
        let cancellation = CancellationToken::new();
        cancellation.cancel();

        let result = until_client_disconnect(&cancellation, async { 42 }).await;

        assert_eq!(result, Ok(42));
        cancellation.cancel();
    }
}
