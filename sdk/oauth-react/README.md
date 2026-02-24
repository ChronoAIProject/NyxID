# @nyxid/oauth-react

React bindings for NyxID OAuth SDK.

## Install

```bash
npm install @nyxid/oauth-react @nyxid/oauth-core
```

## Usage

```tsx
import { NyxIDProvider, createNyxClient, useNyxID } from "@nyxid/oauth-react";

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

## Publish

```bash
npm run prepublishOnly
npm publish
```
