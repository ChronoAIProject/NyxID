# @nyxids/oauth-core

Core OAuth SDK for NyxID.

## Install

```bash
npm install @nyxids/oauth-core
```

## Usage

```ts
import { NyxIDClient } from "@nyxids/oauth-core";

const client = new NyxIDClient({
  baseUrl: "https://auth.example.com",
  clientId: "your-client-id",
  redirectUri: "https://app.example.com/auth/callback",
});

await client.loginWithRedirect();
```

## Connect and call a service

```ts
import { NyxServicesClient } from "@nyxids/oauth-core";

const nyx = new NyxServicesClient({
  baseUrl: "https://auth.example.com",
  auth: { apiKey: process.env.NYXID_API_KEY! },
});

const link = await nyx.connectLinks.create({
  serviceSlug: "github",
  label: "deployment agent",
});

console.info(`Open ${link.connect_url} to connect GitHub`);
const connected = await nyx.connectLinks.waitForCompletion(link.id);

const response = await nyx.services.request(
  connected.slug,
  "/repos/example/project/issues",
  { query: { state: "open" } },
);
const issues = await response.json();
```

`auth` also accepts `{ accessToken }` from an OAuth login. Connect-link
credential entry and provider consent remain browser-only; the SDK creates and
polls the link but never handles the user's external credential.

## Publish

```bash
npm run prepublishOnly
npm publish
```
