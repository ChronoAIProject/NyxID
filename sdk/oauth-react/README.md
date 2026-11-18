# @nyxids/oauth-react

React bindings for NyxID OAuth SDK.

## Install

```bash
npm install @nyxids/oauth-react @nyxids/oauth-core
```

## Usage

```tsx
import { NyxIDProvider, createNyxClient, useNyxID } from "@nyxids/oauth-react";

const client = createNyxClient({
  baseUrl: "https://auth.example.com",
  clientId: "your-client-id",
  redirectUri: "https://app.example.com/auth/callback",
});

function LoginButton() {
  const { loginWithRedirect } = useNyxID();
  return <button onClick={() => void loginWithRedirect()}>Sign in</button>;
}

export function AppRoot() {
  return (
    <NyxIDProvider client={client}>
      <LoginButton />
    </NyxIDProvider>
  );
}
```

`useNyxID()` exposes `getAccessToken()` and `refreshToken()`. The provider runs
an expiry-aware refresh check while it is mounted, and applications should use
`getAccessToken()` before authenticated API calls so rotated refresh tokens are
used automatically. The context observes cross-tab token changes and clears
when a definitive `invalid_grant` removes the token set; transient failures
retain the session. The timer and storage listener are removed on unmount.
Shared localStorage refresh uses Web Locks. In browsers without Web Locks,
configure `storage: window.sessionStorage`; otherwise refresh retains tokens
and raises retryable `refresh_coordination_unavailable` instead of racing another
tab. See the core README for retry/backoff and custom-storage requirements.

To request an additional allowed OAuth scope or service after the user has
already authorized the app, pass the core SDK's incremental options through
the same hook:

```tsx
function AddAccessButton() {
  const { loginWithRedirect } = useNyxID();
  return (
    <button onClick={() => void loginWithRedirect({
      includeGrantedScopes: true,
      scope: "email",
      requestedServiceIds: ["USER_SERVICE_UUID"],
    })}>
      Add access
    </button>
  );
}
```

Omit `scope` for a service-only request or `requestedServiceIds` for a
scope-only request. Handle the callback with the core SDK and verify the
resulting grant before enabling the new capability.

## Publish

```bash
npm run prepublishOnly
npm publish
```
