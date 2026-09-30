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
