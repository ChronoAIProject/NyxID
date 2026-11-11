# NyxID OAuth React Demo

Minimal demo app that uses `@nyxids/oauth-react` to authenticate against NyxID.

## What it demonstrates

- Redirect login with PKCE
- OAuth callback handling and token exchange
- Calling `/oauth/userinfo` using the SDK
- Local session clearing

## Prerequisites

1. Run NyxID backend (`http://localhost:3001` by default).
2. Create a developer OAuth app in NyxID UI.
3. Add this callback URI to that app:
   - `http://localhost:4173/auth/callback`

## Setup

```bash
cd sdk
npm install
cp demo-react/.env.example demo-react/.env
```

Edit `demo-react/.env`:

```bash
VITE_NYXID_BASE_URL=http://localhost:3001
VITE_NYXID_CLIENT_ID=<your-client-id>
VITE_NYXID_REDIRECT_URI=http://localhost:4173/auth/callback
```

## Run

```bash
cd sdk/demo-react
npm run dev
```

Open `http://localhost:4173`.

The demo uses SDK 0.9.0's expiry-aware `getAccessToken()` before UserInfo calls.
The provider checks expiry on mount and every 30 seconds, persists rotated
refresh tokens, and synchronizes other tabs through storage events. A retryable
refresh error keeps the current session; `invalid_grant` requires login again.
Shared browser storage requires Web Locks; configure sessionStorage for a
tab-local demo in older browsers.
