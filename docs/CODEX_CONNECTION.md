# Codex Credential Connection

`nyxid provider connect-codex` optionally imports an explicitly approved OpenAI
API key from local Codex file storage. Installation and generic unattended flags
do not authorize reading configuration or credentials. Skipping performs no
network requests or Codex file reads. No production credentials are needed for
the fixture-based tests.

## Connect

1. Install and verify the CLI and Codex skill registration through
   [the canonical manifest](../skills/INSTALL.md). Check
   `nyxid provider connect-codex --help`; the published initial `v0.15.0` did not
   have this command. Upgrade to a supporting release or use AI Services for
   separate provider authorization when it is unavailable.
2. Select the NyxID API instance. Hosted: `https://nyx-api.chrono-ai.fun`.
   For authorized agent-assisted login, run
   `nyxid login --no-wait --output json --base-url <URL>`, present the human code
   and bare verification URL, then `nyxid login resume <REQUEST_ID> --output json`.
   Provider setup requires a first-party account session. A restricted Agent Key
   cannot import provider credentials.
3. Run `nyxid provider connect-codex --base-url <URL> --output json`. This fetches
   the live account and connection status without opening local Codex files.
   Show the normalized instance, account UUID/email, purpose and existing
   connection ID/version. Explain that replacing an API-key connection updates
   linked legacy services, and verification makes a small paid OpenAI Responses
   request. Obtain explicit approval of the destination and account.
4. Run the local helper using the approved nonsecret fields:

   ```bash
   nyxid provider connect-codex --base-url <URL> \
     --approve-instance <URL> --approve-account <ACCOUNT_UUID> --output json
   ```

   Replacing a reviewed connection additionally requires
   `--replace-connection <CONNECTION_UUID> --replace-version <VERSION>`.
   A changed version requires new review and consent. Never pass the API key in
   command arguments, inspect auth files, print environment variables, or copy
   credentials into an agent conversation.
5. Inspect the redacted result. `saved` means stored but not verified; `usable`
   means this exact saved credential completed a Responses request.
   `reconnect_required` indicates a dead credential or unavailable binding.
   Retry the saved credential with `nyxid provider connect-codex --verify`;
   `--verification-model <MODEL>` selects an available Responses model (default
   `gpt-4.1-mini`). Network errors never establish readiness.

Resume later with `nyxid provider connect-codex`, inspect with `--status`, or skip
with `--skip`. **AI Services > Codex connection** exposes verification and links
to the saved service and separate Codex authorization. Cancelling leaves the CLI
and installed skills usable.

## Storage And Compatibility

The helper reads `${CODEX_HOME:-~/.codex}/config.toml` only after transfer consent.
`cli_auth_credentials_store = "file"` (including the default) supports a bounded,
regular `auth.json` file containing an `OPENAI_API_KEY`. Symlinks, inaccessible
configuration, oversized files and unsupported formats fail closed. Only a
genuinely absent config uses the default. The local store is never modified.

`keyring` and `auto` return a safe separate-authorization fallback. They do not
silently read a file while the active credential may reside in the OS store.
Missing credentials, OS-store denial, expired/revoked credentials and failed
uploads offer retry, separate provider authorization or Skip.

A ChatGPT/Codex OAuth session is not an OpenAI API key. Official Codex CI guidance
requires one machine or serialized job stream per `auth.json`: another consumer
can rotate the refresh token and break the first consumer. It excludes generic
OAuth clients outside Codex. NyxID therefore never imports that shared session;
authorize **OpenAI Codex** separately in AI Services. This has its own expiration,
refresh, reconnect and disconnect lifecycle without sharing local refresh state.

API-key imports use the active `openai` API-key provider and encrypted,
user-owned provider storage. Provisioning and replacement are transactional;
user replacement advances each affected unified credential's epoch, even for
the same key value. Verification never changes that epoch. It uses the normal
metered proxy route with an exact service selector, consumes a bounded completed
Responses result, and binds the result to the token version, key epoch, service
version and endpoint. Another personal/org connection cannot satisfy the test.
Upload redirects are disabled; errors and results expose no credential contents.

Disable pauses a service. Delete removes NyxID's saved credential and endpoint,
not the upstream API key, so local Codex keeps working. Revoking the API key at
OpenAI affects both consumers. Explicit reconnect after a deleted/revoked
connection creates a fresh usable service without re-enabling the prior service
or reviving unrelated revoked keys.

Official references, checked 2026-09-09:

- [Codex authentication and storage](https://developers.openai.com/codex/auth)
- [CI authentication and refresh rotation](https://learn.chatgpt.com/docs/auth/ci-cd-auth)
- [Codex user skill locations](https://developers.openai.com/codex/skills)

The current user skill location is `~/.agents/skills`; `CODEX_HOME` configures
credentials and configuration, not that canonical skill root.
