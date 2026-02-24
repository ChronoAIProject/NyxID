# @nyxid/oauth-core

Core OAuth SDK for NyxID.

## Install

```bash
npm install @nyxid/oauth-core
```

## Usage

```ts
import { NyxIDClient } from "@nyxid/oauth-core";

const client = new NyxIDClient({
  baseUrl: "https://auth.example.com",
  clientId: "your-client-id",
  redirectUri: "https://app.example.com/auth/callback",
});

await client.loginWithRedirect();
```

## Publish

```bash
npm run prepublishOnly
npm publish
```
