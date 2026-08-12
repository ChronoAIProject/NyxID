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

## Triggers and webhook verification

```ts
import {
  NyxServicesClient,
  verifyTriggerWebhookSignature,
} from "@nyxids/oauth-core";

const nyx = new NyxServicesClient({
  baseUrl: "https://auth.example.com",
  auth: { apiKey: process.env.NYXID_API_KEY! },
});

const created = await nyx.triggers.create({
  label: "Repository activity",
  verification: { mode: "token", location: "bearer" },
  delivery: { type: "webhook", url: "https://app.example.com/events" },
});

// Configure the provider to POST to created.trigger.inbound_url using the
// one-time created.secret. The ingress URL is server-to-server, not an SDK API.
// Store delivery_signing_secret under delivery_signing_key_id for outbound
// webhook verification.

const deliveries = await nyx.triggers.listDeliveries(created.trigger.id);
const failed = deliveries.deliveries.find(
  (delivery) => delivery.status === "failed" && delivery.replay_available,
);
if (failed) {
  await nyx.triggers.redeliver(created.trigger.id, failed.event_id);
}

const rotated = await nyx.triggers.rotateDeliverySecret(created.trigger.id);
// Keep both receiver secrets until X-NyxID-Key-Id shows rotated.key_id.
```

For outbound connection or trigger webhooks, verify the signature before
parsing the request body:

```ts
const receiverSecrets: Record<string, string> = await loadReceiverSecrets();
const valid = await verifyTriggerWebhookSignature({
  keyId: request.headers.get("X-NyxID-Key-Id") ?? "",
  secretsByKeyId: receiverSecrets,
  timestamp: request.headers.get("X-NyxID-Timestamp") ?? "",
  signatureHeader: request.headers.get("X-NyxID-Signature") ?? "",
  rawBody: await request.text(),
  toleranceSeconds: 300,
});

if (!valid) return new Response("Invalid signature", { status: 401 });
```

`verifyConnectionWebhookSignature` uses the same timestamp-bound HMAC-SHA256
contract. Both helpers use Web Crypto, work in Node 18+ and edge/browser
runtimes, enforce a five-minute replay window by default, and return `false`
instead of throwing on malformed or mismatched signatures. Passing `secret`
directly remains supported for single-key receivers.

## Publish

```bash
npm run prepublishOnly
npm publish
```
