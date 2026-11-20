//! Inventory of direct dials that must retain the addresses approved for the request.
//! The wire tests cover ordinary forwarding. Proof and admission branches select
//! their transport independently and must execute with the same pinned client.
#[test]
fn guarded_http_execution_keeps_the_request_client() {
    let proxy = include_str!("proxy_service.rs");
    let (_, body) = proxy
        .split_once("async fn forward_request_with_egress_guard_inner(")
        .unwrap();
    let (body, _) = body.split_once("\n}").unwrap();
    assert!(!body.contains("no_redirect_http_client().execute("));
}

#[test]
fn websocket_handshake_uses_an_established_socket() {
    let proxy = include_str!("../handlers/proxy.rs");
    let (_, body) = proxy.split_once("async fn connect_downstream_ws(").unwrap();
    let (body, _) = body.split_once("\n}").unwrap();
    assert!(!body.contains("tokio_tungstenite::connect_async"));
}

#[test]
fn cma_catalog_dispatch_keeps_the_approved_destination() {
    let source = include_str!("assistant_cma_gateway.rs");
    let (runtime, _) = source.split_once("#[cfg(test)]\nmod tests").unwrap();
    assert!(!runtime.contains("GATEWAY_HTTP_CLIENT"));
    assert!(!runtime.contains("tokio_tungstenite::connect_async"));
}
