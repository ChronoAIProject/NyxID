---
name: nyxid-service-skill-authoring
description: Find or author an agent skill for a NyxID proxy service that has no OpenAPI spec or typed operations. Use when a NyxID service only exposes the generic proxy tool, `nyxid catalog endpoints` returns nothing for the slug, or you would otherwise have to guess endpoint paths. Searches Ornn for an existing skill bound to the service; if none exists, creates one — researching the official OpenAPI spec on the web for public services, or collecting the contract from the user for private/custom services (never fabricate endpoints) — then uploads it to Ornn, binds it to the service, and records it locally.
version: "1.1"
metadata:
  category: plain
  tag:
    - nyxid
    - proxy
    - skill-authoring
---

# NyxID service skill authoring (via Ornn)

You are an agent facing a NyxID proxy service you do not know how to call:
the service publishes no OpenAPI spec, `nyxid catalog endpoints <slug>`
returns nothing, and the only tool available is the generic proxy
`request` (path + method + body). **Do not guess endpoint paths from
training memory** — that is how hallucinated calls happen. Follow this
flow instead: find an existing skill on Ornn, or author one from a
verifiable source.

Prerequisite: the `ornn-agent-manual-cli` skill (the Ornn API contract).
If it is not already in your context or local skills directory, pull it
first:

```bash
nyxid proxy request ornn-api "/api/v1/skills/ornn-agent-manual-cli/json" \
  --method GET --output json
```

All Ornn calls below assume that manual is loaded; it defines
authentication, response shapes, and error handling.

## No shell? Every step works over plain HTTP

Some agent runtimes cannot execute a CLI but can make HTTP requests. Every
`nyxid` command in this skill has a direct REST equivalent against the
NyxID API (base URL below is the deployment you authenticate against,
e.g. `https://nyx-api.chrono-ai.fun`):

- **Auth**: send `Authorization: Bearer <token>` on every request — a
  NyxID agent API key (`nyxid_ag_...`) or a user access token. Ask the
  user for a key if you have none; never scrape one from disk.
- **Ornn calls**: `nyxid proxy request ornn-api "<path>" --method M`
  is exactly `M {BASE}/api/v1/proxy/s/ornn-api<path>` — same path, same
  body, same JSON response. This holds for every Ornn endpoint in this
  skill and in the Ornn manual.
- **Service inventory** (`nyxid service list`):
  `GET {BASE}/api/v1/keys` — the authoritative connected-services view.
- **Catalog metadata** (`nyxid catalog show <slug>`):
  `GET {BASE}/api/v1/catalog/{slug}`; typed operations
  (`nyxid catalog endpoints <slug>`):
  `GET {BASE}/api/v1/catalog/{slug}/endpoints`.
- **Mount a spec / set skills** (`nyxid service update ...`):
  `PUT {BASE}/api/v1/keys/{user_service_id}` with a JSON body — e.g.
  `{"openapi_spec_url": "https://..."}` (empty string clears) or
  `{"recommended_skills": ["name"]}` (empty list clears). The endpoint
  document alone can also be updated via
  `PUT {BASE}/api/v1/endpoints/{endpoint_id}` with the same fields.
- **Call the target service through the proxy**
  (`nyxid proxy request <slug> ...`):
  `M {BASE}/api/v1/proxy/s/{slug}/<path>` — NyxID injects the stored
  credential; never send the downstream credential yourself.

The remainder of this skill shows CLI forms for brevity; translate with
the table above when you are HTTP-only.

## Step 0 — Confirm the service really has no contract

```bash
nyxid service list --output json          # find the service, note slug + source (catalog|custom)
nyxid catalog show <slug> --output json   # catalog metadata: openapi_spec_url, homepage_url, recommended_skills
nyxid catalog endpoints <slug>            # typed operations, if any
```

- If `endpoints` returns operations, or `openapi_spec_url` is set: stop —
  use the typed operations/spec directly. This skill is for the spec-less
  case only.
- If `recommended_skills` is non-empty: those are curated pointers for
  this service. Look each one up on Ornn (Step 1) before anything else.

## Step 1 — Search Ornn for an existing skill

Check `~/.ornn/installed-skills.json` first, then search Ornn. Try
service-bound skills before free-text:

```bash
# Skills bound to NyxID services (system skills), grouped by service
nyxid proxy request ornn-api "/api/v1/skill-facets/system-services" \
  --method GET --output json

# Free-text: try up to 5 query variations (slug, vendor name, API name)
nyxid proxy request ornn-api \
  "/api/v1/skill-search?query=<service-name>&mode=semantic&scope=mixed&pageSize=20" \
  --method GET --output json
```

If a match exists: pull it with `GET /api/v1/skills/<name>/json`, install
it locally, record it in `~/.ornn/installed-skills.json`, and follow it.
Done — do not author a duplicate.

## Step 2 — Classify the service before authoring

Look at the service's origin (`source` field from `nyxid service list`,
plus `nyxid catalog show`):

- **Public/official service**: catalog-backed, or a custom endpoint whose
  base URL is a well-known public API host with a vendor homepage. Its
  contract is publicly documented → go to Step 3A.
- **User-created/private service**: custom endpoint on an internal host,
  private IP, unknown domain, or anything clearly built by the user or
  their org. Its contract is NOT publicly knowable → go to Step 3B.
  **Never author endpoints for a private service from guesswork or from
  "similar" public APIs.**

When unsure, treat the service as private and ask.

## Step 3A — Public service: research the official contract

1. Search the web for the vendor's official OpenAPI spec and API
  reference (try `<vendor> openapi.json`, `<vendor> API reference`,
  the vendor's GitHub org, `homepage_url` from the catalog).
2. **Official machine-readable spec found (JSON, reachable, ≤5 MB)** —
  the best outcome is no skill at all: mount it so NyxID publishes typed
  operations for every consumer:

   ```bash
   nyxid service update <slug> --openapi-spec-url <official-spec-url>
   nyxid catalog endpoints <slug>    # verify operations appeared
   ```

   Only add a skill on top if the API has non-obvious usage rules (auth
   quirks, pagination idioms, ordering constraints) worth teaching.
3. **Spec exists but is unusable directly (YAML, >5 MB, split across
  files) or there is no machine-readable spec** — author a skill from
  the official documentation:
  - `SKILL.md` documenting: what the service is, the proxy call shape
    (`nyxid proxy request <slug> "<path>" --method ... --data ...`),
    the 5-15 most useful operations with method, path, required
    parameters, one worked example each, and known limitations. Cite
    the official doc URLs you used — every operation must trace to
    vendor documentation, not memory.
  - Verify at least one read-only operation live through the proxy
    before publishing, when the user's credentials allow it.
4. If the service is part of the NyxID **official catalog** and deserves
  typed operations platform-wide, also suggest to the user that a curated
  overlay be added to the NyxID repo (`backend/specs/catalog/`) — that is
  the platform-managed path; your Ornn skill covers the gap meanwhile.

## Step 3B — Private/custom service: the user is the only source

Ask the user, in one batch, for whatever they can provide:

1. What the service is and what it is used for.
2. An OpenAPI/Swagger document or URL, if one exists (mount it with
  `nyxid service update <slug> --openapi-spec-url ...` — often this
  alone removes the need for a skill).
3. Otherwise: endpoint list (method + path + parameters), sample
  requests/responses, and any usage rules.

Author the skill strictly from what they provide. If the user cannot
provide the contract, **stop and say so**: a skill cannot be created for
a service whose interface nobody can describe. Do not fill gaps with
invented endpoints.

Default the skill to **private** on Ornn (that is also Ornn's default).
Only share/publish it if the user asks; a private service's skill may
reveal internal API structure.

## Step 4 — Package, validate, upload, bind

Follow the Ornn manual (§2.1 step 5) exactly:

1. `GET /api/v1/skill-format/rules` — read the package rules.
2. Write the package: one root folder named after the skill,
  `SKILL.md` with quoted `version: "1.0"` frontmatter.
3. `POST /api/v1/skill-format/validate` with the ZIP — loop until
  `valid: true`.
4. `POST /api/v1/skills` — upload. Keep the returned `guid`.
5. **Bind the skill to the NyxID service** so future agents find it from
  the service itself:

   ```bash
   nyxid proxy request ornn-api "/api/v1/skills/<guid>/nyxid-service" \
     --method PUT \
     --data '{"nyxidServiceId":"<service-id from /api/v1/me/nyxid-services>"}' \
     --output json
   ```

   Binding to an admin (platform) service makes it a public system skill;
   binding to your own personal service keeps your chosen visibility.
6. Install the skill locally and record it in
  `~/.ornn/installed-skills.json`, then execute the user's original task
  with it.

## Discovery notes for future sessions

- NyxID catalog entries expose `recommended_skills`; platform admins can
  set it (admin service update) so agents jump straight from a catalog
  entry to the right skill. If you authored a skill for a catalog
  service, suggest the admin add its name there.
- Ornn's `GET /api/v1/skill-facets/system-services` lists every
  service-bound skill platform-wide — always check it before authoring.
