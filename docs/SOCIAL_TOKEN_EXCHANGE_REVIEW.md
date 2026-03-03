# Social Token Exchange - Code & Security Review

**Reviewer:** Code Review Agent
**Date:** 2026-03-03
**Branch:** feat/social-token-exchange
**Status:** Review Complete

---

## 1. Summary

The social token exchange feature adds RFC 8693-style token exchange for external social providers (Google ID tokens and GitHub access tokens). The implementation introduces two new files (`crypto/jwks.rs`, `services/social_token_exchange_service.rs`), two new error variants, modifications to the OAuth token handler, and a `JwksCache` in `AppState`.

**Overall Assessment:** The implementation is **solid and well-structured**. It follows existing codebase patterns consistently, has good test coverage for utility functions, and addresses the most critical security concerns (algorithm pinning, JWKS URL hardcoding, email verification enforcement, audience validation). The issues found are mostly MEDIUM/LOW severity with two HIGH items that should be addressed before merge.

---

## 2. Critical Issues

**None found.** No show-stopping security vulnerabilities or data loss risks were identified.

---

## 3. High Issues

### H1. GitHub access tokens are accepted under `id_token` subject_token_type

**File:** `backend/src/handlers/oauth.rs:843-877`
**Severity:** HIGH (semantic/security inconsistency)

The handler routes the social token exchange under `subject_token_type = "urn:ietf:params:oauth:token-type:id_token"`, but GitHub tokens received in this flow are **access tokens**, not ID tokens. The `provider` field disambiguates the behavior, but:

1. A GitHub access token is opaque and validated by calling GitHub's API (not by cryptographic verification). This is semantically different from an ID token exchange.
2. RFC 8693 defines `urn:ietf:params:oauth:token-type:access_token` for access tokens. Overloading the `id_token` type creates ambiguity.

**Impact:** Clients sending GitHub tokens must lie about the `subject_token_type`. Future providers could be confused about which type to use.

**Suggested Fix:** Either:
- (a) Accept both `id_token` and `access_token` subject_token_types for the social exchange path (using `provider` as the real discriminator), OR
- (b) Add a dedicated `subject_token_type` like `urn:ietf:params:oauth:token-type:access_token` as an additional match arm that also dispatches to the social exchange when `provider` is present, to disambiguate from the existing delegation flow.

```rust
// Option (b): Add a second match arm in the token handler
"urn:ietf:params:oauth:token-type:id_token"
| "urn:ietf:params:oauth:token-type:access_token"
    if body.provider.is_some() =>
{
    // Social token exchange (provider is present)
    ...
}
"urn:ietf:params:oauth:token-type:access_token" => {
    // Existing delegation flow (no provider)
    ...
}
```

### H2. No audit log on social token exchange failure

**File:** `backend/src/services/social_token_exchange_service.rs:43-115`
**Severity:** HIGH (security monitoring gap)

The existing delegation token exchange (`token_exchange_service.rs`) logs failures via `log_exchange_failure()`. The social token exchange only logs **successful** exchanges (line 94-105). Failed attempts (invalid token, wrong provider, verification failure) produce no audit record.

**Impact:** Brute-force or credential-stuffing attacks against the social token exchange endpoint leave no audit trail. Incident response teams cannot detect attack patterns.

**Suggested Fix:** Add failure audit logging for each error path. Example:

```rust
// At the top of exchange_social_token, wrap the main logic in a closure or
// add audit logging in the error paths:
let result = do_exchange(...).await;
if let Err(ref e) = result {
    audit_service::log_async(
        db.clone(),
        None,
        "social_token_exchange_failed".to_string(),
        Some(serde_json::json!({
            "provider": provider,
            "client_id": client_id,
            "error": format!("{e}"),
        })),
        None,
        None,
    );
}
result
```

---

## 4. Medium Issues

### M1. JWKS cache stampede on concurrent requests

**File:** `backend/src/crypto/jwks.rs:79-100`
**Severity:** MEDIUM (reliability)

When the JWKS cache expires, the `get_keys()` method drops the read lock, then every concurrent request will proceed to call `fetch_and_cache()` simultaneously. With high concurrency this creates a "thundering herd" / cache stampede hitting Google's JWKS endpoint repeatedly.

**Impact:** Under high load, many redundant JWKS fetches could trigger rate limiting from Google or add latency.

**Suggested Fix:** Use a "single-flight" pattern. One approach with `tokio::sync::RwLock`:

```rust
// In fetch_and_cache, acquire write lock FIRST, then double-check freshness:
async fn fetch_and_cache(&self, jwks_uri: &str) -> AppResult<Vec<CachedKey>> {
    // Double-checked locking: re-check under write lock
    let mut cache = self.inner.write().await;
    if let Some(entry) = cache.get(jwks_uri) {
        if entry.fetched_at.elapsed() < entry.max_age {
            return Ok(clone_keys(&entry.keys));
        }
    }
    // Proceed with fetch while holding write lock (or release and use a
    // dedicated Mutex/Notify per URI)
    ...
}
```

### M2. `#[allow(dead_code)]` on `SocialTokenExchangeResponse`

**File:** `backend/src/services/social_token_exchange_service.rs:14`
**Severity:** MEDIUM (code quality)

The `#[allow(dead_code)]` annotation suppresses warnings for the entire struct. If this struct is being used (and it is -- in `exchange_social_token`), the annotation is unnecessary. If specific fields like `user_id` are only used internally but the struct fields are constructed, it should not need suppression.

**Suggested Fix:** Remove `#[allow(dead_code)]`. If `user_id` triggers a warning because it is only written but never read externally by the handler, consider either:
- Reading it in the handler (the handler currently ignores `result.user_id`), or
- Removing the field and keeping `user_id` as a local variable in the service only.

### M3. `#[allow(clippy::too_many_arguments)]` on exchange_social_token

**File:** `backend/src/services/social_token_exchange_service.rs:32`
**Severity:** MEDIUM (code quality / maintainability)

The function takes 9 arguments. While the existing codebase has similar patterns, this is an opportunity to bundle related parameters.

**Suggested Fix:** Consider a request context struct:

```rust
pub struct SocialExchangeContext<'a> {
    pub db: &'a mongodb::Database,
    pub config: &'a AppConfig,
    pub jwt_keys: &'a JwtKeys,
    pub jwks_cache: &'a JwksCache,
    pub http_client: &'a reqwest::Client,
}
```

This reduces the function to 4-5 arguments. However, this is a stylistic choice and the existing pattern in the codebase uses individual arguments, so deferring is acceptable.

### M4. `iat` freshness check uses server clock without skew tolerance

**File:** `backend/src/crypto/jwks.rs:261-266`
**Severity:** MEDIUM (reliability)

The `validate_google_claims` function rejects tokens where `now - claims.iat > 600` seconds. However, there is no tolerance for clock skew between Google's servers and the NyxID server. If the NyxID server's clock is ahead by even a few seconds, recently issued tokens could be unnecessarily rejected. Note that `jsonwebtoken`'s built-in `exp` validation has a default 60-second leeway, but the `iat` check does not.

**Impact:** Occasional false rejections in environments with clock drift.

**Suggested Fix:** Add a small skew allowance (e.g., 30 seconds) to the iat check:

```rust
const CLOCK_SKEW_TOLERANCE_SECS: i64 = 30;
if now - claims.iat > GOOGLE_ID_TOKEN_MAX_AGE_SECS + CLOCK_SKEW_TOLERANCE_SECS {
    return Err(...);
}
```

### M5. Future `iat` values are not rejected

**File:** `backend/src/crypto/jwks.rs:259-266`
**Severity:** MEDIUM (security hardening)

The `iat` freshness check only validates that `now - claims.iat <= 600`. If `claims.iat` is in the future (e.g., due to a crafted token), the subtraction yields a negative value which is always `<= 600`, so it passes. Tokens with future `iat` should be rejected.

**Suggested Fix:**

```rust
fn validate_google_claims(claims: GoogleIdTokenClaims) -> AppResult<GoogleIdTokenClaims> {
    let now = chrono::Utc::now().timestamp();

    // Reject future iat (with small skew tolerance)
    if claims.iat > now + 30 {
        return Err(AppError::ExternalTokenInvalid(
            "Token issued_at is in the future".to_string(),
        ));
    }

    // Reject tokens older than max age
    if now - claims.iat > GOOGLE_ID_TOKEN_MAX_AGE_SECS {
        return Err(AppError::ExternalTokenInvalid(
            "Token is too old (iat exceeds maximum age)".to_string(),
        ));
    }
    // ...
}
```

### M6. Frontend type `subject_token_type` is hardcoded to `id_token` only

**File:** `frontend/src/types/api.ts:296`
**Severity:** MEDIUM (consistency with H1)

The `SocialTokenExchangeRequest` type constrains `subject_token_type` to `"urn:ietf:params:oauth:token-type:id_token"`. If H1 is fixed to also accept `access_token` type for GitHub, this frontend type needs to be updated.

**Suggested Fix:** Update the type to a union if both types are supported:

```typescript
readonly subject_token_type:
  | "urn:ietf:params:oauth:token-type:id_token"
  | "urn:ietf:params:oauth:token-type:access_token";
```

---

## 5. Low Issues

### L1. Error message includes user-supplied provider name

**File:** `backend/src/services/social_token_exchange_service.rs:49`
**Severity:** LOW (minor info leak)

```rust
AppError::ExternalProviderNotConfigured(format!("Unsupported provider: {provider}"))
```

This reflects the user-supplied `provider` value back in the error message. While the `provider` field is likely a short string and the risk is minimal (no XSS since this is a JSON API), it is slightly inconsistent with the codebase pattern of using generic error messages.

**Suggested Fix:** Use a generic message:

```rust
AppError::ExternalProviderNotConfigured("Unsupported or unconfigured provider".to_string())
```

### L2. Error message in `verify_with_keys` includes `kid` value

**File:** `backend/src/crypto/jwks.rs:223-226`
**Severity:** LOW (minor info leak)

The kid value comes from the JWT header (user-controlled input). Including it in the error message is low risk since it is just a key identifier string, but it is worth noting.

### L3. `parse_jwk` accepts keys with no `alg` field as RS256

**File:** `backend/src/crypto/jwks.rs:290-292`
**Severity:** LOW (defense in depth)

```rust
let alg = match jwk.alg.as_deref() {
    Some("RS256") | None => Algorithm::RS256,
    _ => return None,
};
```

If a JWK has no `alg` field, it defaults to RS256. This is reasonable for Google's JWKS (which always specifies `alg`), but as a defense-in-depth measure, it might be safer to require `alg` to be explicitly `RS256`.

**Suggested Fix:** Change to require explicit algorithm:

```rust
let alg = match jwk.alg.as_deref() {
    Some("RS256") => Algorithm::RS256,
    _ => return None,
};
```

### L4. `expect()` in `verify_with_keys` function

**File:** `backend/src/crypto/jwks.rs:243`
**Severity:** LOW (code quality)

```rust
let err = last_err.expect("at least one key was tried");
```

This `expect` is logically safe (guarded by the `matching_keys.is_empty()` check above), but in production code, using `unwrap_or_else` with an explicit error is slightly more defensive:

**Suggested Fix:**

```rust
let err = last_err.unwrap_or_else(|| {
    jsonwebtoken::errors::Error::from(jsonwebtoken::errors::ErrorKind::InvalidToken)
});
```

### L5. `DatabaseError` variant used as error code 1007

**File:** `backend/src/errors/mod.rs:177` (existing, not new)
**Severity:** LOW (pre-existing)

The `DatabaseError` variant shares the same 1007 code range. The new `ExternalTokenInvalid` (6004) and `ExternalProviderNotConfigured` (6005) error codes are properly unique and well-placed in the 6xxx social auth range. No issue with the new code.

### L6. Token endpoint is on the public OAuth router (no per-IP rate limiting)

**File:** `backend/src/routes.rs:344-357`, `backend/src/main.rs:258-264`
**Severity:** LOW (defense in depth)

The `/oauth/token` endpoint is on `public_oauth` which has open CORS. The global rate limiter is applied to the entire app (line 261-264 in main.rs), so the token endpoint IS rate limited. However, the social token exchange could benefit from stricter per-endpoint rate limiting since it involves external API calls (JWKS fetch, GitHub API) that could be abused.

**Suggested Fix:** Consider adding a tighter rate limit specifically for the token exchange path, or ensuring the per-IP rate limiter covers the public OAuth routes. This is a future enhancement, not a blocker.

---

## 6. Positive Observations

The following security controls are correctly implemented:

1. **Algorithm pinning:** RS256 is enforced; other algorithms including `none` are rejected (`jwks.rs:180-185`).
2. **JWKS URL hardcoding:** Only `GOOGLE_JWKS_URI` constant is used; no user-controlled JWKS URLs, preventing SSRF (`jwks.rs:10`).
3. **Issuer validation:** Hardcoded `GOOGLE_ISSUER` constant used in validation, not user input (`jwks.rs:11, 232`).
4. **Audience validation:** Validated against configured `GOOGLE_CLIENT_ID` from env vars (`jwks.rs:233, social_token_exchange_service.rs:123-127`).
5. **Email verification enforced:** Google tokens without `email_verified: true` are rejected (`social_token_exchange_service.rs:134-138`). GitHub uses API-verified emails via `/user/emails`.
6. **Constant-time secret comparison:** Client secret validation uses `subtle::ConstantTimeEq` (`oauth_service.rs:138-140`).
7. **Error message sanitization:** Internal/database errors never leak to clients (`errors/mod.rs:309-311`).
8. **OAuth error format:** Token endpoint errors use RFC 6749 Section 5.2 format with proper `error` codes for new variants (`errors/mod.rs:221-222`).
9. **Cache TTL bounds:** JWKS cache TTL is clamped between 5 minutes and 24 hours (`jwks.rs:13-14`), preventing both too-frequent fetches and stale keys.
10. **Key rotation handling:** JWKS cache performs a force-refresh retry on verification failure, handling Google key rotation gracefully (`jwks.rs:199-202`).
11. **Encryption key filtering:** Only RSA signing keys are accepted from JWKS; `enc` use keys are rejected (`jwks.rs:286-287`).
12. **Audit logging:** Successful exchanges are audit-logged with provider and client_id (`social_token_exchange_service.rs:94-105`).
13. **Layer separation:** Clean handler -> service -> crypto layering maintained.
14. **Test coverage:** Good unit tests for `parse_cache_control_max_age`, `parse_jwk`, `validate_google_claims`, and provider parsing.
15. **No token/secret logging:** No `tracing` calls log the actual token values.

---

## 7. File-by-File Summary

| File | Verdict | Notes |
|------|---------|-------|
| `crypto/jwks.rs` | Good | Well-structured JWKS cache with proper security controls. Issues: M1 (stampede), M4/M5 (iat checks), L3 (alg default), L4 (expect) |
| `services/social_token_exchange_service.rs` | Good | Clean orchestration. Issues: H2 (no failure audit), M2 (dead_code), M3 (args), L1 (error message) |
| `handlers/oauth.rs` | Good | Proper integration. Issue: H1 (token type semantics) |
| `errors/mod.rs` | Good | New variants are well-placed with unique error codes and proper OAuth error mapping |
| `crypto/mod.rs` | Good | Simple module registration, no issues |
| `services/mod.rs` | Good | Simple module registration, no issues |
| `main.rs` | Good | JwksCache properly initialized with shared HTTP client |
| `frontend/src/types/api.ts` | Good | Clean type definitions. Issue: M6 (coupled to H1) |

---

## 8. Recommendations Summary

**Must fix before merge:**
- H2: Add audit logging for failed social token exchange attempts

**Should fix before merge:**
- H1: Address the `subject_token_type` semantic mismatch for GitHub tokens
- M5: Reject future `iat` values in Google token validation

**Should fix (can be follow-up):**
- M1: JWKS cache stampede prevention
- M4: Clock skew tolerance for `iat` check
- M2: Remove `#[allow(dead_code)]`

**Nice to fix:**
- L1-L6: Minor improvements noted above
- M3: Consider parameter bundling
- M6: Frontend type update (dependent on H1)
