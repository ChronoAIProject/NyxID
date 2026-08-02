# Buffered Settlement Response Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Preserve the status, headers, and body of a successful buffered downstream response when post-forward billing settlement fails.

**Architecture:** Reuse NyxID's existing deferred settlement path at the shared buffered proxy response branch. Extend the existing Mongo-backed mounted-route regression test so the current synchronous branch demonstrably returns `500`, then replace that single synchronous call with `settle_meter_async` and prove the same request returns the downstream `200` body while the failed settlement remains recoverable.

**Tech Stack:** Rust, Axum 0.8, Tokio, MongoDB 8.0, existing NyxID billing integration harness.

## Global Constraints

- Billing is metadata-only after forwarding and must not rewrite a successful downstream response.
- Reuse `settle_meter_async`; add no new billing abstraction or production-only injection point.
- Add no MIME or ZIP special case, retry, Ornn change, or Aevatar fallback.
- Keep the change to the shared buffered response call and one focused behavioral test.

---

## File Map

- `backend/src/billing_integration_tests.rs`: mounted buffered-route failure/recovery regression test and existing controlled downstream fixture.
- `backend/src/handlers/proxy.rs`: shared buffered response settlement call site.

### Task 1: Preserve the Buffered Response

**Files:**
- Modify: `backend/src/billing_integration_tests.rs`
- Modify: `backend/src/handlers/proxy.rs`

**Interfaces:**
- Consumes: existing `start_controlled_billing_downstream()`, mounted private proxy router, Mongo-backed billing rows, and `settle_meter_async(Arc<BillingService>, MeteredProxyContext, PlatformUsage, Option<ResaleUsage>, Option<String>)`.
- Produces: mounted buffered proxy requests return the exact downstream success status/body even when settlement fails; the existing reconciler can still finish the durable settlement after the wallet is restored.

- [x] **Step 1: Write the failing behavioral test**

Rename `mounted_route_settlement_failure_is_replayed_once_by_reconcile` to `buffered_route_preserves_success_when_settlement_failure_is_replayed` and change the response assertions immediately after releasing the controlled downstream:

```rust
let response = route.await.expect("mounted recovery route task");
assert_eq!(response.status(), StatusCode::OK);
let body = to_bytes(response.into_body(), usize::MAX)
    .await
    .expect("consume controlled downstream response");
assert_eq!(body.as_ref(), br#"{"ok":true}"#);

let failed = wait_for_route_usage_status(&db, &service.slug, UsageStatus::Failed).await;
assert!(failed.forwarded);
assert!(!failed.released);
assert_eq!(failed.quantity, Some(1));
```

Replace the three calls to `usage_row_for_service` in this test with a test-only condition helper so the deferred failure is observed without depending on Tokio scheduling:

```rust
async fn wait_for_route_usage_status(
    db: &mongodb::Database,
    service_slug: &str,
    expected: UsageStatus,
) -> UsageMeterRow {
    for _ in 0..100 {
        let row = usage_row_for_service(db, service_slug).await;
        if row.status == expected {
            return row;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    panic!("route {service_slug} did not reach {expected:?}");
}
```

Use `UsageStatus::Forwarded` before deleting the wallet, `UsageStatus::Failed` before restoring it, and `UsageStatus::Finalized` after reconciliation. Keep the existing wallet restore, forced retry timestamp, `run_once()`, released assertion, idempotent second reconcile, and single-row assertion. This catches the production bug: restoring the current awaited `BillingService::settle` call must make the test fail by returning a NyxID `500` instead of the literal downstream response.

- [x] **Step 2: Run the regression test and verify RED**

Run:

```bash
cargo test -p nyxid buffered_route_preserves_success_when_settlement_failure_is_replayed -- --nocapture
```

Expected: FAIL at `assert_eq!(response.status(), StatusCode::OK)` with actual status `500 Internal Server Error`. If MongoDB is unavailable and the test skips, start the repository's MongoDB test dependency and rerun; a skipped test is not RED.

- [x] **Step 3: Implement the minimal shared fix**

In the buffered response branch of `execute_proxy`, replace the awaited synchronous settlement call:

```rust
state
    .billing
    .settle(
        &metered,
        llm_platform_usage(reported_usage.as_ref(), request_len + response_len),
        resale,
        model,
    )
    .await?;
```

with the already-used deferred helper:

```rust
settle_meter_async(
    state.billing.clone(),
    metered,
    llm_platform_usage(reported_usage.as_ref(), request_len + response_len),
    resale,
    model,
)
.await;
```

Do not change streaming, billing service, retry, or response-building code.

- [x] **Step 4: Run the regression test and verify GREEN**

Run:

```bash
cargo test -p nyxid buffered_route_preserves_success_when_settlement_failure_is_replayed -- --nocapture
```

Expected: PASS. The response is the literal downstream `200` body; the meter row records a retryable settlement failure and the explicit reconciler invocation finalizes it exactly once.

- [x] **Step 5: Run focused billing regressions**

Run:

```bash
cargo test -p nyxid billing_route_coverage_smoke -- --nocapture
cargo test -p nyxid billing_service_lifecycle_regression -- --nocapture
```

Expected: both PASS with MongoDB-backed assertions executed.

- [x] **Step 6: Run repository verification**

Run:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo nextest run -p nyxid --profile ci --test-threads 2
cargo nextest run --workspace --exclude nyxid --profile ci
bash scripts/check-rci-backend-boundary.sh
git diff --check
```

Expected: every command exits `0`; the backend JUnit report records 4,912 tests with zero failures, the remaining workspace report records 1,082 tests with zero failures, and the worktree contains only the planned test, proxy, spec, and plan changes.

- [x] **Step 7: Commit the verified fix**

```bash
git add backend/src/billing_integration_tests.rs backend/src/handlers/proxy.rs docs/superpowers/plans/2026-08-02-buffered-settlement-response.md
git diff --cached --check
git commit -m "fix(proxy): preserve buffered responses on settlement failure"
```

Do not push until the user explicitly selects the NyxID remote branch or PR destination.
