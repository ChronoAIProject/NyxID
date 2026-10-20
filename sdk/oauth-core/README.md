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

To request more access after an existing NyxID grant, use the same redirect
and callback flow. `scope` contains only the new OAuth permissions; omit it
when only adding services. The new scope must be allowed for the client, and
service IDs are exact NyxID UserService UUIDs.

```ts
await client.loginWithRedirect({
  includeGrantedScopes: true,
  scope: "email",
  requestedServiceIds: ["USER_SERVICE_UUID"],
});
```

The SDK generates a random `state` and `handleRedirectCallback()` verifies it
on success and error callbacks. If you need to restore a draft, generate a fresh
random state, store the draft reference under it in your app, and pass that
state to `loginWithRedirect()`. Exchange a successful code with
`handleRedirectCallback()`, then re-read the effective grant before enabling
the new access. An error or cancellation does not prove that access changed.
Existing scopes and services are retained by NyxID.

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
