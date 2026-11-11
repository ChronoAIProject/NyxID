# SDK Workspace

## Structure

```
sdk/
|-- package.json
|-- oauth-core/
|   |-- src/index.ts
|   |-- src/services.ts
|   |-- test/services.test.ts
|   |-- package.json
|   |-- tsconfig.json
|   `-- README.md
|-- oauth-react/
|   |-- src/index.tsx
|   |-- package.json
|   |-- tsconfig.json
|   `-- README.md
`-- demo-react/
    |-- src/main.tsx
    |-- src/App.tsx
    |-- package.json
    |-- .env.example
    `-- README.md
```

## Responsibilities

- `sdk/package.json`: workspace root, unified build and publish-precheck scripts.
- `oauth-core`: protocol and PKCE primitives, token handling, connect-link orchestration, and service proxy calls.
- `oauth-react`: React bindings layered on top of core primitives.
- `demo-react`: runnable reference client showing end-to-end OAuth redirect flow.

## Dependency Boundary

- `oauth-react` depends on `@nyxids/oauth-core` via workspace linkage.
- `demo-react` depends on `@nyxids/oauth-react` via workspace linkage.
- SDK workspace remains independent from app runtime code in `frontend/`.

## Refresh lifecycle

OAuth SDK 0.9.0 records `expiresAt` with the token set. `getAccessToken()`
refreshes within 60 seconds of expiry; `refreshToken()` explicitly rotates the
refresh credential. A single storage write publishes the complete successor.
Concurrent callers sharing a storage object share one promise. Browser Web Locks
serialize across documents and a waiter adopts an already persisted successor.
Without Web Locks, persistent localStorage refresh fails closed with retryable
`refresh_coordination_unavailable`; supply sessionStorage for tab-local use.
Custom storage adapters must not share credentials across tabs without Web Locks.
No expiring localStorage lease is used: it cannot provide atomic admission.

`invalid_grant` clears only the obsolete token set and signals re-login. Network
errors, `temporarily_unavailable`, and 503 retain tokens and retry up to three
times with backoff and `Retry-After`. Refresh responses cannot undo logout or
replace a newer login. The React provider synchronizes explicit calls, polls
expiry every 30 seconds while mounted, observes cross-tab storage changes, and
removes its timer/listener on unmount. The demo uses the expiry-aware accessor
before fetching userinfo.
