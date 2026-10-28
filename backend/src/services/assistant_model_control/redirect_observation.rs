//! Request-scoped facts from the actual selected client's redirect policy.
//! Policy actions are delegated unchanged. No headers, URLs or replay are added.
use std::{ops::Deref, sync::{Arc, atomic::{AtomicBool, Ordering}}};

#[derive(Clone)]
pub struct HttpClient(Arc<ClientInner>);
struct ClientInner {
    client: reqwest::Client,
    brand: Option<Arc<Brand>>,
}
struct Brand;

tokio::task_local! {
    static TRACE: Trace;
}
#[derive(Clone)]
struct Trace {
    brand: Arc<Brand>,
    attempted: Arc<AtomicBool>,
    mismatched: Arc<AtomicBool>,
}

impl HttpClient {
    /// The marker can only be constructed together with this immutable client
    /// and its decorated policy. The caller supplies its ORIGINAL policy.
    pub(crate) fn observed(builder: reqwest::ClientBuilder, original: reqwest::redirect::Policy) -> Result<Self, reqwest::Error> {
        let brand = Arc::new(Brand);
        let policy_brand = brand.clone();
        let policy = reqwest::redirect::Policy::custom(move |attempt| {
            let _ = TRACE.try_with(|trace| {
                if Arc::ptr_eq(&trace.brand, &policy_brand) {
                    // Attempt.previous is reqwest's actual policy-time history.
                    // Conservatively exclude even a redirect that policy stops.
                    trace.attempted.store(true, Ordering::SeqCst);
                } else { trace.mismatched.store(true, Ordering::SeqCst); }
            });
            original.redirect(attempt)
        });
        let client = builder.redirect(policy).build()?;
        Ok(Self(Arc::new(ClientInner {client, brand:Some(brand)})))
    }

    /// Raw clients retain ordinary behavior but cannot attest zero redirects.
    pub(crate) fn unobserved(client: reqwest::Client) -> Self {
        Self(Arc::new(ClientInner {client, brand:None}))
    }
    pub(crate) fn as_client(&self) -> &reqwest::Client { &self.0.client }

    pub(super) fn probe(&self, actual: &reqwest::Client) -> Option<Probe> {
        if !std::ptr::eq(actual, self.as_client()) { return None; }
        Some(Probe {client:self.clone(), trace:Trace {brand:self.0.brand.clone()?, attempted:Arc::new(AtomicBool::new(false)), mismatched:Arc::new(AtomicBool::new(false))}})
    }
}
impl Deref for HttpClient {
    type Target = reqwest::Client;
    fn deref(&self) -> &Self::Target { self.as_client() }
}

pub(crate) struct Probe {
    client: HttpClient,
    trace: Trace,
}
impl Probe {
    // reqwest 0.12.28 PendingRequest polls its redirect-layer response future
    // directly. Its TowerRedirectPolicy invokes the original policy in that
    // same execute future. Only success returns evidence; dropping/canceling
    // this consuming future cannot produce a completed direct observation.
    pub(crate) async fn execute(self, request: reqwest::Request) -> Result<(reqwest::Response, bool), reqwest::Error> {
        let response = TRACE.scope(self.trace.clone(), self.client.as_client().execute(request)).await?;
        let direct = !self.trace.attempted.load(Ordering::SeqCst)
            && !self.trace.mismatched.load(Ordering::SeqCst);
        Ok((response, direct))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{Router, routing::get, response::IntoResponse, http::StatusCode};

    #[tokio::test]
    async fn exact_client_provenance_and_same_final_url_redirect_are_distinct() {
        let count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let seen=count.clone();
        let app=Router::new().route("/a",get(move || {let seen=seen.clone();async move {
            if seen.fetch_add(1,Ordering::SeqCst)==0 { (StatusCode::TEMPORARY_REDIRECT,[("location","/b")],"redirect").into_response() }
            else { (StatusCode::OK,"final a").into_response() }
        }})).route("/b",get(||async { (StatusCode::TEMPORARY_REDIRECT,[("location","/a")],"back") }));
        let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url=format!("http://{}/a",listener.local_addr().unwrap());
        let server=tokio::spawn(async move {axum::serve(listener,app).await.unwrap();});
        let client=HttpClient::observed(reqwest::Client::builder(),reqwest::redirect::Policy::limited(10)).unwrap();
        let clone=client.clone();
        let probe=clone.probe(client.as_client()).unwrap();
        let (response,direct)=probe.execute(client.get(&url).build().unwrap()).await.unwrap();
        assert_eq!(response.url().as_str(),url);
        assert_eq!(response.text().await.unwrap(),"final a");
        assert_eq!(count.load(Ordering::SeqCst),2);
        assert!(!direct);
        let raw_clone=client.as_client().clone();
        assert!(client.probe(&raw_clone).is_none());
        let plain=HttpClient::unobserved(reqwest::Client::new());
        assert!(plain.probe(plain.as_client()).is_none());
        let direct=client.probe(client.as_client()).unwrap();
        let (response,direct)=direct.execute(client.get(&url).build().unwrap()).await.unwrap();
        assert_eq!(response.status(),StatusCode::OK);
        assert!(direct);
        server.abort();
    }

    #[tokio::test]
    async fn decorating_a_stop_policy_preserves_its_actual_action() {
        let count=Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let reached=count.clone();
        let app=Router::new().route("/a",get(||async { (StatusCode::TEMPORARY_REDIRECT,[("location","/b")]) }))
            .route("/b",get(move ||{let reached=reached.clone();async move {reached.fetch_add(1,Ordering::SeqCst);StatusCode::OK}}));
        let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url=format!("http://{}/a",listener.local_addr().unwrap());
        let server=tokio::spawn(async move {axum::serve(listener,app).await.unwrap();});
        let client=HttpClient::observed(reqwest::Client::builder(),reqwest::redirect::Policy::none()).unwrap();
        let probe=client.probe(client.as_client()).unwrap();
        let (response,direct)=probe.execute(client.get(url).build().unwrap()).await.unwrap();
        assert_eq!(response.status(),StatusCode::TEMPORARY_REDIRECT);
        assert!(!direct);
        assert_eq!(count.load(Ordering::SeqCst),0);
        server.abort();
    }

    #[tokio::test]
    async fn consuming_execute_isolates_concurrency_nested_scope_and_cancellation() {
        let started=Arc::new(tokio::sync::Notify::new());
        let waiting=started.clone();
        let app=Router::new()
            .route("/direct",get(||async {StatusCode::OK}))
            .route("/redirect",get(||async {(StatusCode::TEMPORARY_REDIRECT,[("location","/direct")])}))
            .route("/pending",get(move || {let waiting=waiting.clone();async move {
                waiting.notify_one();
                std::future::pending::<StatusCode>().await
            }}));
        let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base=format!("http://{}",listener.local_addr().unwrap());
        let server=tokio::spawn(async move {axum::serve(listener,app).await.unwrap();});
        let client=HttpClient::observed(reqwest::Client::builder(),reqwest::redirect::Policy::limited(10)).unwrap();
        let direct=client.probe(client.as_client()).unwrap();
        let redirected=client.probe(client.as_client()).unwrap();
        let unrelated=Trace {brand:Arc::new(Brand),attempted:Arc::new(AtomicBool::new(false)),mismatched:Arc::new(AtomicBool::new(false))};
        let (a,b)=TRACE.scope(unrelated.clone(),async {
            tokio::join!(direct.execute(client.get(format!("{base}/direct")).build().unwrap()),
                redirected.execute(client.get(format!("{base}/redirect")).build().unwrap()))
        }).await;
        assert!(a.unwrap().1);
        assert!(!b.unwrap().1);
        assert!(!unrelated.attempted.load(Ordering::SeqCst));
        assert!(!unrelated.mismatched.load(Ordering::SeqCst));
        let probe=client.probe(client.as_client()).unwrap();
        let request=client.get(format!("{base}/pending")).build().unwrap();
        let pending=tokio::spawn(probe.execute(request));
        started.notified().await;
        pending.abort();
        assert!(pending.await.unwrap_err().is_cancelled());
        // A fresh exact execute is independent; cancellation returned no
        // response/evidence and there is no separate queryable direct flag.
        let probe=client.probe(client.as_client()).unwrap();
        assert!(probe.execute(client.get(format!("{base}/direct")).build().unwrap()).await.unwrap().1);
        server.abort();
    }
}
