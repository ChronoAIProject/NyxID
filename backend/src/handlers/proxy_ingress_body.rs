//! Fingerprint bounded bodies while retaining their original frames for forwarding.
use std::{
    collections::VecDeque,
    pin::Pin,
    task::{Context, Poll},
};

use axum::{
    body::Body,
    http::{Request, header},
};
use bytes::Bytes;
use http_body_util::BodyExt;
use hyper::body::{Body as HttpBody, Frame, SizeHint};
use sha2::{Digest, Sha256};

const MAX_BYTES: u64 = 64 * 1024;
// Bound metadata too: hostile streams may yield arbitrarily many empty frames.
const MAX_FRAMES: usize = 1024;

pub(super) async fn fingerprint(request: &mut Request<Body>) -> Option<[u8; 32]> {
    if request.headers().contains_key(header::TRANSFER_ENCODING) {
        return None;
    }
    let mut lengths = request.headers().get_all(header::CONTENT_LENGTH).iter();
    let length = match lengths.next() {
        Some(value) => {
            let value = value.to_str().ok()?;
            if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
                return None;
            }
            let length = value.parse::<u64>().ok()?;
            if length > MAX_BYTES || lengths.next().is_some() {
                return None;
            }
            length
        }
        None if request.body().is_end_stream() => return Some(Sha256::digest([]).into()),
        None => return None,
    };
    if request.body().is_end_stream() {
        return (length == 0).then(|| Sha256::digest([]).into());
    }
    let mut tail = std::mem::replace(request.body_mut(), Body::empty());
    let mut saved = VecDeque::new();
    let mut digest = Sha256::new();
    let mut bytes = 0;
    let eligible = loop {
        if saved.len() == MAX_FRAMES {
            break false;
        }
        let hint = tail.size_hint();
        let end = tail.is_end_stream();
        let Some(frame) = tail.frame().await else {
            break bytes == length;
        };
        let valid = match &frame {
            Ok(frame) => {
                if let Some(data) = frame.data_ref() {
                    bytes += data.len() as u64;
                    if bytes <= length {
                        digest.update(data);
                    }
                }
                bytes <= length
            }
            Err(_) => false,
        };
        saved.push_back(SavedFrame { frame, hint, end });
        if !valid {
            break false;
        }
    };
    *request.body_mut() = Body::new(Replay { saved, tail });
    eligible.then(|| digest.finalize().into())
}

struct SavedFrame {
    frame: Result<Frame<Bytes>, axum::Error>,
    hint: SizeHint,
    end: bool,
}

struct Replay {
    saved: VecDeque<SavedFrame>,
    tail: Body,
}

impl HttpBody for Replay {
    type Data = Bytes;
    type Error = axum::Error;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, Self::Error>>> {
        if let Some(saved) = self.saved.pop_front() {
            Poll::Ready(Some(saved.frame))
        } else {
            Pin::new(&mut self.tail).poll_frame(cx)
        }
    }

    fn is_end_stream(&self) -> bool {
        self.saved
            .front()
            .map_or_else(|| self.tail.is_end_stream(), |saved| saved.end)
    }

    fn size_hint(&self) -> SizeHint {
        self.saved
            .front()
            .map_or_else(|| self.tail.size_hint(), |saved| saved.hint.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    #[tokio::test]
    async fn refusal_throttle_round2_replays_frames_hints_headers_and_trailers_without_copying() {
        let first = Bytes::from_static(b"hello ");
        let second = Bytes::from_static(b"world");
        let pointers = [first.as_ptr(), second.as_ptr()];
        let trailers = axum::http::HeaderMap::from_iter([(
            axum::http::HeaderName::from_static("x-test-trailer"),
            "original".parse().unwrap(),
        )]);
        let frames = [
            Ok::<_, std::io::Error>(Frame::data(first)),
            Ok(Frame::data(second)),
            Ok(Frame::trailers(trailers.clone())),
        ];
        let body = Body::new(http_body_util::StreamBody::new(futures::stream::iter(
            frames,
        )));
        let mut request = Request::builder()
            .header("content-length", "11")
            .header("content-encoding", "identity")
            .body(body)
            .unwrap();
        let headers = request.headers().clone();
        let hint = request.body().size_hint();
        let end = request.body().is_end_stream();
        assert_eq!(
            fingerprint(&mut request).await,
            Some(Sha256::digest(b"hello world").into())
        );
        assert_eq!(request.headers(), &headers);
        assert_eq!(request.body().size_hint().lower(), hint.lower());
        assert_eq!(request.body().size_hint().upper(), hint.upper());
        assert_eq!(request.body().is_end_stream(), end);
        for pointer in pointers {
            let data = request
                .body_mut()
                .frame()
                .await
                .unwrap()
                .unwrap()
                .into_data()
                .unwrap();
            assert_eq!(data.as_ptr(), pointer, "payload must be moved, not copied");
        }
        assert_eq!(
            request
                .body_mut()
                .frame()
                .await
                .unwrap()
                .unwrap()
                .into_trailers()
                .unwrap(),
            trailers
        );
        assert!(request.body_mut().frame().await.is_none());
    }

    #[tokio::test]
    async fn refusal_throttle_round2_invalid_lengths_errors_and_frame_churn_replay_without_counting()
     {
        for (declared, actual) in [(3, "four"), (5, "four")] {
            let mut request = Request::builder()
                .header("content-length", declared)
                .body(Body::from(actual))
                .unwrap();
            assert_eq!(fingerprint(&mut request).await, None);
            assert_eq!(
                axum::body::to_bytes(request.into_body(), 100)
                    .await
                    .unwrap(),
                actual
            );
        }
        let frames = futures::stream::iter([
            Ok::<_, std::io::Error>(Bytes::from_static(b"prefix")),
            Err(std::io::Error::other("synthetic read error")),
        ]);
        let mut request = Request::builder()
            .header("content-length", 12)
            .body(Body::from_stream(frames))
            .unwrap();
        assert_eq!(fingerprint(&mut request).await, None);
        assert_eq!(
            request
                .body_mut()
                .frame()
                .await
                .unwrap()
                .unwrap()
                .data_ref()
                .unwrap(),
            b"prefix".as_slice()
        );
        assert!(request.body_mut().frame().await.unwrap().is_err());
        let polls = Arc::new(AtomicUsize::new(0));
        let counter = polls.clone();
        let stream = futures::stream::repeat_with(move || {
            counter.fetch_add(1, Ordering::SeqCst);
            Ok::<_, std::io::Error>(Bytes::new())
        });
        let mut request = Request::builder()
            .header("content-length", 1)
            .body(Body::from_stream(stream))
            .unwrap();
        assert_eq!(fingerprint(&mut request).await, None);
        assert_eq!(polls.load(Ordering::SeqCst), MAX_FRAMES);
        for _ in 0..MAX_FRAMES {
            assert!(
                request
                    .body_mut()
                    .frame()
                    .await
                    .unwrap()
                    .unwrap()
                    .data_ref()
                    .unwrap()
                    .is_empty()
            );
        }
        assert_eq!(
            polls.load(Ordering::SeqCst),
            MAX_FRAMES,
            "replayed prefix must not repoll the stream"
        );
    }

    #[tokio::test]
    async fn refusal_throttle_round2_ineligible_bodies_are_never_polled() {
        for length in [None, Some("65537"), Some("garbage"), Some("-1")] {
            let polls = Arc::new(AtomicUsize::new(0));
            let counter = polls.clone();
            let body = Body::from_stream(futures::stream::poll_fn(move |_| {
                counter.fetch_add(1, Ordering::SeqCst);
                Poll::Pending::<Option<Result<Bytes, std::io::Error>>>
            }));
            let mut builder = Request::builder();
            if let Some(length) = length {
                builder = builder.header("content-length", length);
            }
            let mut request = builder.body(body).unwrap();
            assert_eq!(fingerprint(&mut request).await, None);
            assert_eq!(polls.load(Ordering::SeqCst), 0);
        }
        let mut empty = Request::new(Body::empty());
        assert_eq!(
            fingerprint(&mut empty).await,
            Some(Sha256::digest([]).into())
        );
        let mut boundary = Request::builder()
            .header("content-length", MAX_BYTES)
            .body(Body::from(vec![0; MAX_BYTES as usize]))
            .unwrap();
        assert!(fingerprint(&mut boundary).await.is_some());
    }

    #[tokio::test]
    async fn refusal_throttle_round2_small_body_cost_observation() {
        const BODY: &[u8] = br#"{"workspace_id":"synthetic"}"#;
        const ITERATIONS: usize = 10_000;
        let mut elapsed = [std::time::Duration::ZERO; 2];
        for (enabled, duration) in [false, true].into_iter().zip(&mut elapsed) {
            let started = std::time::Instant::now();
            for _ in 0..ITERATIONS {
                let mut request = Request::builder()
                    .header("content-length", BODY.len())
                    .body(Body::from(BODY))
                    .unwrap();
                if enabled {
                    std::hint::black_box(fingerprint(&mut request).await.unwrap());
                }
                std::hint::black_box(
                    axum::body::to_bytes(request.into_body(), 65536)
                        .await
                        .unwrap(),
                );
            }
            *duration = started.elapsed();
        }
        eprintln!(
            "refusal throttle small-body observation: bytes={} iterations={ITERATIONS} baseline_ns/op={} fingerprint_ns/op={} delta_ns/op={}; payload_copies=0, single_frame_allocations=2 (frame deque + boxed replay), frame_metadata_bytes={}",
            BODY.len(),
            elapsed[0].as_nanos() / ITERATIONS as u128,
            elapsed[1].as_nanos() / ITERATIONS as u128,
            elapsed[1].as_nanos() as i128 / ITERATIONS as i128
                - elapsed[0].as_nanos() as i128 / ITERATIONS as i128,
            std::mem::size_of::<SavedFrame>()
        );
    }
}
