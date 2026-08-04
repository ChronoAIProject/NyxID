# Release Integrity

NyxID's remote credential accept flow uses a standalone backend-served page with
Subresource Integrity (SRI) on the bundled JavaScript and an optional signed
release manifest URL configured by the deployment host.

This is detection, not prevention. It gives an administrator a fingerprint to
compare out-of-band against an independently published manifest before entering
a credential. It does not prevent server-side HTML substitution, and it is not a
defense against a T1 node public-key substitution MITM.

## Runtime Configuration

The backend exposes:

```json
{
  "api_base_url": "https://auth.example.invalid",
  "release_integrity": {
    "enabled": true,
    "manifest_url": "https://example.invalid/releases.json",
    "verification_ttl_secs": 1800
  }
}
```

`RELEASE_INTEGRITY_MANIFEST_URL` is host configuration. When it is unset or
blank, `release_integrity.enabled` is `false` and `manifest_url` is `null`.
For owners that have not explicitly opted out by org policy, the browser and
backend fail closed when the manifest URL is unset.

`verification_ttl_secs` is `JWT_RELAY_REPLY_TTL_SECS`.

## Standalone Accept Page

The frontend build emits:

- `frontend/dist/credential-accept/credential-accept.html`
- `frontend/dist/credential-accept/assets/credential-accept-*.js`
- `frontend/dist/release-integrity/releases.json`

The backend handler serves the HTML at:

- `/nodes/{node_id}/credentials/pending/{pending_id}/accept`
- `/nodes/credentials/pending/{pending_id}/fan-out/accept`

The backend also serves the standalone bundle assets at:

- `/credential-accept/assets/*`

It reads only the prebuilt HTML and release manifest, verifies that manifest
script artifacts have matching `sha384-*` SRI attributes in the HTML, and emits
a strict CSP. The backend does not read credential plaintext and does not
perform remote credential cryptography.

In split deployments where a reverse proxy sends the SPA/static frontend and
backend to different upstreams, route these accept paths and
`/credential-accept/assets/*` to the backend upstream, not the SPA/static
frontend. Frontend callers build absolute accept URLs from
`runtime-config.api_base_url`, so that value must be the externally reachable
backend origin for browser users.

## Manifest Schema

`frontend/dist/release-integrity/releases.json` has schema version
`nyxid.release-integrity.v1`:

```json
{
  "schema_version": "nyxid.release-integrity.v1",
  "app_version": "0.6.0",
  "git_commit": "abc123",
  "generated_at": "2026-06-05T00:00:00.000Z",
  "credential_accept": {
    "fingerprint_sha384_hex": "96 lowercase hex chars"
  },
  "artifacts": [
    {
      "role": "credential_accept_html",
      "path": "/credential-accept/credential-accept.html",
      "content_type": "text/html; charset=utf-8",
      "size_bytes": 1234,
      "sha384_sri": "sha384-...",
      "sha384_hex": "96 lowercase hex chars"
    },
    {
      "role": "credential_accept_script",
      "path": "/credential-accept/assets/credential-accept-abc.js",
      "content_type": "text/javascript; charset=utf-8",
      "size_bytes": 1234,
      "sha384_sri": "sha384-...",
      "sha384_hex": "96 lowercase hex chars"
    }
  ]
}
```

`sha384_sri` is `sha384-` plus RFC 4648 standard base64 with padding of the raw
48-byte SHA-384 digest. `sha384_hex` is the same digest encoded as lowercase
hex.

The credential accept fingerprint is SHA-384 hex over this byte sequence:

```text
ASCII "nyxid:rci-accept:v1\0"
for each credential_accept_script artifact sorted by UTF-8 path byte order:
  u32be(path_utf8_len)
  path_utf8
  u64be(script_byte_len)
  script_bytes
```

The page displays the first 12 hex characters for the admin to compare against
the independently published manifest.

## Submission Metadata

The encrypted ciphertext POST may include:

```json
{
  "integrity_verification": {
    "mode": "admin_verified",
    "fingerprint_sha384_hex": "96 lowercase hex chars",
    "verified_at": "2026-06-05T00:00:00.000Z",
    "manifest_url_configured": true
  }
}
```

or, for org-policy opt-out:

```json
{
  "integrity_verification": {
    "mode": "org_policy_opt_out",
    "fingerprint_sha384_hex": null,
    "verified_at": null,
    "manifest_url_configured": false
  }
}
```

This metadata is an audit and UX gate only. It is not part of HKDF, AAD, nonce
selection, ciphertext, node WebSocket payloads, or stored pending credential
documents. Audit logs store only the first 12 hex characters of
`fingerprint_sha384_hex`.

## Org Opt-Out

Org owners can opt out at:

`User.profile_config.release_integrity.remote_credential_integrity_verification_opt_out`

The flag is effective only for `user_type=Org`. Missing `profile_config`,
missing `release_integrity`, and missing flag values default to `false`.

## Release Workflow

The release workflow builds and uploads the generated manifest as a workflow
artifact. Signing and publication are disabled unless
`RELEASE_INTEGRITY_SIGNING_ENABLED=true`.

When enabled, maintainers must provide:

- `RELEASE_INTEGRITY_SIGN_CMD`
- `RELEASE_INTEGRITY_PUBLISH_CMD`
- `RELEASE_INTEGRITY_MANIFEST_URL`

Optional secret material can be provided via:

- `RELEASE_INTEGRITY_SIGNING_KEY`
- `RELEASE_INTEGRITY_PUBLISH_TOKEN`

Signing workflow behavior is:

- Disabled or unset `RELEASE_INTEGRITY_SIGNING_ENABLED`: warn and skip signing
  and publication.
- Enabled with all required command and URL configuration: sign and publish.
- Enabled with missing required configuration: fail fast and name the missing
  variables. This surfaces a deployment misconfiguration instead of silently
  publishing an unsigned manifest that an operator expected to be signed.

No default release origin, CDN, signing key, or rotation policy is assumed.

## Maintainer Runbook TBD

- [ ] TBD, owner Infra: signing key custody and storage.
- [ ] TBD, owner Security: signing key rotation and authority policy.
- [ ] TBD, owner Infra: independent release origin DNS, CDN, and TLS
  ownership.
- [ ] TBD, owner Security: final signature scheme and verification procedure.
- [ ] TBD, owner Release-eng: manifest signing and publish process.
- [ ] TBD, owner Release-eng: manifest validity, expiry, and stale-manifest
  handling.
