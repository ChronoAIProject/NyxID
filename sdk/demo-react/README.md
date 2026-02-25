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
