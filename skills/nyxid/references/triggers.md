# Inbound triggers and signed webhook delivery

Use a NyxID trigger when an external sender can emit an event but the receiver needs a stable
verification, signing, deduplication, or replay contract. This is not a credential-backed outbound
service call.

## Create the authority boundary

NyxID v0.10+ exposes trigger management in the CLI:

```bash
nyxid trigger create \
  --label "Lark Base to Aevatar" \
  --verification bearer \
  --delivery webhook \
  --delivery-url "https://receiver.example.com/events" \
  --output json
```

Verification choices are `bearer`, `query`, and `hmac`. Prefer bearer when a sender can set a
static header, or HMAC when it can sign the exact raw body. Query tokens are a last resort because
URLs leak through logs, history, monitoring, and referrers. Webhook delivery URLs must use HTTPS
and resolve only to public addresses.

Create returns two different one-time secrets:

- `secret` authenticates the sender at `trigger.inbound_url`. Put it only in the sender's bearer
  header, query parameter, or HMAC signer selected at creation.
- `delivery_signing_secret` authenticates NyxID to the webhook receiver. Put it only in the
  receiver's secret store. Select it during rotation with `delivery_signing_key_id` /
  `X-NyxID-Key-Id`.

Never print either secret into chat, logs, workflow YAML, issue text, or URLs. A credential dedicated
to one automation can establish that automation as the source; a shared credential proves only that
one of its holders sent the request.

## Ingress and delivery contracts

Send an event to the returned inbound URL with a stable event id:

```http
POST <trigger.inbound_url>
Authorization: Bearer <inbound-secret>
X-NyxID-Event-Id: base-record-change-123
Content-Type: application/json

{"record_id":"rec-123","name":"Ada","department":"Engineering"}
```

NyxID delivers a JSON envelope containing `event_id`, `trigger_id`, `source`, `received_at`, and
the original body under `payload`. Outbound webhook headers are:

- `X-NyxID-Delivery-Id`: stable event id for receiver dedupe.
- `X-NyxID-Timestamp`: Unix seconds.
- `X-NyxID-Signature`: `sha256=<hex>` over the exact bytes
  `timestamp + "." + raw_request_body`.
- `X-NyxID-Key-Id`: selects the current delivery secret.

Verify timestamp skew, compute HMAC-SHA256 over the exact wire bytes, and compare in constant time
before mapping `payload` fields into application input. Caller authentication does not by itself
prove data freshness; read the source system again only when the event omitted required fields, a
newer state is required, or a separate business rule demands it.

## Observe and recover

Webhook delivery has durable cross-replica admission/deduplication and up to three bounded attempts.
Ingress returns `accepted` before background delivery completes. Inspect the same delivery instead
of sending the business event again:

```bash
nyxid trigger deliveries <trigger-id> --output json
nyxid trigger redeliver <trigger-id> <event-id> --output json
nyxid trigger rotate-delivery-secret <trigger-id> --output json
```

Retained webhook envelopes are encrypted and expire after the configured retention window (72 hours
by default). Delivery lists contain metadata only. A deployment with retention set to `0` cannot
redeliver payloads. Keep the receiver idempotent even with durable admission.

For Aevatar's host-managed `/api/workflow-webhooks/{routeKey}` bridge, configure its signature,
timestamp, and delivery-id header names to the NyxID headers above and map original fields from
`payload.<field>`. That Host binding remains operator-managed; trigger creation does not create it.
The current binding stores one HMAC secret and does not select a two-key map from
`X-NyxID-Key-Id`, so coordinate its configuration change when rotating the delivery secret.
