# Device-Code Grant: Headless Device Provisioning

## Table of contents

- [When to use this reference](#when-to-use-this-reference)
- [Device-code grant vs provider device-code OAuth](#device-code-grant-vs-provider-device-code-oauth)
- [Provisioning flow](#provisioning-flow)
- [CLI commands](#cli-commands)
- [API endpoints](#api-endpoints)
- [Approve-time service and org access](#approve-time-service-and-org-access)
- [Factory keys](#factory-keys)
- [`nyxprov://` QR onboarding](#nyxprov-qr-onboarding)
- [Deferred paths and limitations](#deferred-paths-and-limitations)

## When to use this reference

Use this page when the user is provisioning a headless device, approving a device code shown on hardware, building ESP32 or IoT first-boot firmware, creating factory Ed25519 keys, or generating a `nyxprov://` QR payload for a no-WiFi device.

Device-code grant provisions NyxID operational credentials for a device:

- a scoped NyxID API key with platform `device-code` for signed-poll approval, or `device-onboard` for QR onboarding
- a NyxID node id used as the device's identity and API-key node scope
- a one-time refresh token for future device credential refresh work

These devices do not run `nyxid node register`, do not redeem `nyx_nreg_...` registration tokens, and do not authenticate to `/api/v1/nodes/ws` during this grant flow. The node row is still created so the device has a stable NyxID node identity and the issued API key can be scoped to that node.

## Device-code grant vs provider device-code OAuth

Do not confuse this page with provider OAuth device-code flows.

| Flow | Purpose | CLI / API surface |
|---|---|---|
| Device-code grant | A fresh headless device asks NyxID for its own NyxID API key, node id, and refresh token. | `nyxid device approve`, `nyxid device onboard`, `nyxid device factory-key`, `/devices/code/*`, `/devices/onboard` |
| Provider device-code OAuth | A user connects a downstream provider credential, such as Codex or GitHub, using that provider's RFC 8628 OAuth flow. | `nyxid service add <slug> --device-code`, `/providers/{id}/connect/device-code/*`; see `services.md` / `managing.md` |

When a user says "approve my ESP32", "factory key", "headless device", "nyxprov QR", or gives a 12-character hardware user code, use this reference. When they say "connect Codex/OpenAI/GitHub with device code", use the service/provider references.

## Provisioning flow

Signed device-code grant is RFC 8628-flavored but does not issue OAuth access tokens:

1. Device boots with an Ed25519 private key from factory provisioning.
2. Device calls `POST /api/v1/devices/code/request` with its base64 public key, hardware id, and optional suggested label.
3. NyxID returns `device_code`, `user_code` in `XXXX-XXXX-XXXX` form, `verification_uri`, `verification_uri_complete`, `expires_in`, and `poll_interval`.
4. Device displays the `user_code` or a QR for `verification_uri_complete`.
5. A human opens the bind page or runs `nyxid device approve <CODE>`.
6. The device polls `POST /api/v1/devices/code/poll`, signing each poll with its Ed25519 key.
7. The first successful approved poll returns `api_key`, `node_id`, `refresh_token`, and `expires_in`. NyxID then marks the row delivered and clears the encrypted one-time delivery secrets.

The default binding window is 15 minutes. Pending user codes rotate every 30 seconds; approval accepts only the current displayed generation. Poll every returned `poll_interval` seconds, normally 5 seconds.

## CLI commands

| Goal | Command | Notes |
|---|---|---|
| Approve a displayed device code | `nyxid device approve ABCD-EFGH-JKLM --label "Hall camera"` | Dashes and spaces are optional. The CLI normalizes valid codes to `XXXX-XXXX-XXXX`. |
| Approve under an org | `nyxid device approve ABCD-EFGH-JKLM --org <ID|SLUG|NAME>` | Caller must be allowed to write for that owner; for orgs this normally means org admin. The CLI resolves UUID, slug, or display name before calling the API. |
| Grant services at approval | `nyxid device approve ABCD-EFGH-JKLM --service llm-openai --service api-weather` | Repeatable. Accepts user-service slugs or UUIDs. Without this, the device API key has no service allowlist. |
| QR onboard a no-WiFi device | `WIFI_PASSWORD=... nyxid device onboard --label "Kitchen Camera" --ssid "Home" --password-env WIFI_PASSWORD --service llm-openai` | Prints the raw `nyxprov://` payload to stdout for QR rendering. Summary ids go to stderr. |
| Generate factory keys | `nyxid device factory-key --count 100 --ndjson --out factory-keys.ndjson` | Creates Ed25519 keypairs for firmware burn-in or production-line provisioning. |

`nyxid device onboard` intentionally has no bare `--password` flag. Put the WiFi password in an environment variable so it does not land in shell history or process listings.

## API endpoints

All paths below are under `/api/v1`.

### Request a code

Unauthenticated:

```bash
curl -sS -X POST http://localhost:3001/api/v1/devices/code/request \
  -H "Content-Type: application/json" \
  -d '{
    "device_pubkey": "'"$DEVICE_PUBKEY_BASE64"'",
    "hw_id": "esp32-p4-aabbcc",
    "suggested_label": "Hall camera"
  }'
```

`device_pubkey` is a base64-encoded 32-byte Ed25519 public key. `hw_id` must be 1 to 256 characters. `suggested_label` is optional.

Example response:

```json
{
  "device_code": "m2zC9QEPmhtdfJoEIu5OxjcNbHYUX_ntv5qU3yD78vQ",
  "user_code": "ABCD-EFGH-JKLM",
  "verification_uri": "http://localhost:3000/settings/devices/bind",
  "verification_uri_complete": "http://localhost:3000/settings/devices/bind?user_code=ABCD-EFGH-JKLM",
  "expires_in": 900,
  "poll_interval": 5
}
```

### Poll for approval

Unauthenticated, but each request must carry a valid Ed25519 signature:

```bash
curl -sS -X POST http://localhost:3001/api/v1/devices/code/poll \
  -H "Content-Type: application/json" \
  -d '{
    "device_code": "'"$DEVICE_CODE"'",
    "timestamp": '"$TIMESTAMP"',
    "signature": "'"$SIGNATURE_BASE64"'"
  }'
```

The signature message is:

```text
"nyxid:device-code:poll:v1" || base64url_decode(device_code) || timestamp as signed i64 big-endian bytes
```

Pending response:

```json
{
  "status": "pending",
  "current_user_code": "ABCD-EFGH-JKLM",
  "interval": 5
}
```

Approved response, returned once:

```json
{
  "status": "approved",
  "api_key": "nyxid_ag_...",
  "node_id": "1d16ddfe-89f1-4ec1-a316-4a5c8960d55f",
  "refresh_token": "1fb20ca9a1fd9b10ddfc7d2a5b7d5f54f326e2e6a2ef8e70b8a4fd9e0bd41c47",
  "expires_in": 86400
}
```

A second poll after delivery returns `device_code_already_delivered` with error code `9505`.

### Approve a user code

Authenticated as a human user:

```bash
curl -sS -X POST http://localhost:3001/api/v1/devices/code/approve \
  -H "Authorization: Bearer $ACCESS_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "user_code": "ABCD-EFGH-JKLM",
    "org_id": null,
    "label": "Hall camera",
    "default_services": ["llm-openai"]
  }'
```

The API accepts `org_id` only as a UUID. Prefer the CLI when the user gives an org slug or display name.

Example response:

```json
{
  "device_label": "Hall camera",
  "hw_id": "esp32-p4-aabbcc",
  "api_key_id": "4ae1830c-45c6-4f0d-9f04-0c66c8925a73",
  "node_id": "1d16ddfe-89f1-4ec1-a316-4a5c8960d55f",
  "owner_user_id": "b7e6faee-594b-49a8-91f6-b2c2d20db741",
  "org_id": null
}
```

### Onboard with a QR payload

Authenticated as a human user:

```bash
curl -sS -X POST http://localhost:3001/api/v1/devices/onboard \
  -H "Authorization: Bearer $ACCESS_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "org_id": null,
    "label": "Kitchen Camera",
    "wifi_ssid": "MyHomeNetwork",
    "wifi_password": "hunter22",
    "default_services": ["llm-openai"]
  }'
```

Example response:

```json
{
  "qr_payload": "nyxprov://full?ssid=MyHomeNetwork&psw=hunter22&key=nyxid_ag_...&node=1d16ddfe-89f1-4ec1-a316-4a5c8960d55f&refresh=...&url=https%3A%2F%2Fapi.example.com",
  "node_id": "1d16ddfe-89f1-4ec1-a316-4a5c8960d55f",
  "api_key_id": "4ae1830c-45c6-4f0d-9f04-0c66c8925a73",
  "label": "Kitchen Camera"
}
```

## Approve-time service and org access

`default_services` and CLI `--service` values are resolved against the final owner. They accept user-service UUIDs or slugs and are deduplicated. The created API key has:

- `scopes = "proxy"`
- `allow_all_services = false`
- `allowed_service_ids = <resolved default services>`
- `allow_all_nodes = false`
- `allowed_node_ids = [<created node id>]`
- `platform = "device-code"` for approved signed-poll devices, or `"device-onboard"` for QR-onboarded devices

If no services are supplied, the device gets no proxy service access. Grant service scopes later from the API key management surface if needed.

For personal ownership, omit `--org` / `org_id`. For org ownership, the actor must be able to write for that org owner. Use `nyxid device approve ... --org <ID|SLUG|NAME>` or `nyxid device onboard ... --org <ID|SLUG|NAME>` so the CLI resolves user-friendly org identifiers before calling the backend.

## Factory keys

Generate one Ed25519 keypair:

```bash
nyxid device factory-key
```

Generate production-line NDJSON:

```bash
nyxid device factory-key --count 100 --ndjson --out factory-keys.ndjson
```

Output shape:

```json
[
  {
    "pubkey_hex": "64 lowercase hex chars",
    "privkey_hex": "64 lowercase hex chars"
  }
]
```

On Unix, `--out` writes the file with owner-only permissions (`0600`). The public key must be sent to `/devices/code/request` as base64:

```bash
DEVICE_PUBKEY_BASE64=$(printf '%s' "$PUBKEY_HEX" | xxd -r -p | base64)
```

The private key belongs in the device secure provisioning path or encrypted NVS. Never send it to NyxID and never paste it into chat.

## `nyxprov://` QR onboarding

`nyxid device onboard` is for a device that has no WiFi yet but can scan a QR code from a phone, browser, or printed page. NyxID creates the API key and node immediately, stores only a SHA-256 hash of the generated refresh token, and returns a `nyxprov://full?...` payload containing:

- `ssid`: WiFi SSID
- `psw`: WiFi password
- `key`: raw NyxID API key
- `node`: NyxID node id
- `refresh`: raw one-time refresh token
- `url`: NyxID backend base URL

The WiFi password is used only to build the QR payload. It is not stored in MongoDB, audit logs, or tracing fields.

Pipe stdout into the user's QR renderer of choice:

```bash
WIFI_PASSWORD='hunter22' nyxid device onboard \
  --label "Kitchen Camera" \
  --ssid "MyHomeNetwork" \
  --password-env WIFI_PASSWORD \
  --service llm-openai
```

## Deferred paths and limitations

The server-side grant and QR creation paths are implemented. The device-side consumer for redeeming `nyxprov://` payloads and consuming the onboard refresh token is deferred to follow-up work tracked after #747. Do not document or call a NyxID backend redeem endpoint for `nyxprov://`; it does not exist in this PR.

Rate limits and errors to surface to firmware or operators:

- Device-code endpoints are rate limited per source IP and per known device public key.
- Poll timestamps must be within the allowed skew and must be strictly newer than the previous successful poll timestamp.
- Three consecutive invalid signatures lock the row for one hour.
- Device-code errors reserve `9500-9599`: not found, expired, invalid signature, invalid user code, pending, already delivered, rate limited, locked, and slow down.
