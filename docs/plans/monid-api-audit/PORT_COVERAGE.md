# Port coverage: what moves from Monid to NyxID

Status: planning summary, 2026-10-08. Derived from `migration-backlog-summary.json`, `provider-migration-backlog.csv`, and `summary.json` in this directory. No vendor accounts, runtime changes, or published tools yet.

## Inventory to port

| Surface | Count | Notes |
| --- | ---: | --- |
| Hosted providers (Monid catalog) | 87 | Provider labels, not verified vendor accounts |
| Hosted tools enumerable publicly | 2,440 | Monid reports 2,441; StockAnalysis returns 7 of 8 |
| Source definitions (MIT connector repo) | 699 | Across 31 providers, full schemas and connector code |
| Source definitions matched to hosted tools | 637 | Contract and behavior available |
| Source-only definitions | 62 | Need reconciliation or direct import |
| Hosted-only tools | 1,803 | Interface known, native upstream contract not yet obtained |

## How much of the source is a plain HTTP port

| Source behavior | Count of 699 | Port consequence |
| --- | ---: | --- |
| Plain request candidates | 267 | OpenAPI overlay + existing proxy, no adapter |
| Request transformations | 107 | Adapter or native-contract rewrite |
| Lifecycle hooks (any) | 380 | Review per hook type |
| Polling hooks (async jobs) | 132 | Job ownership, poll, cancel, result access |
| Resource ownership checks | 13 | Local ownership records for shared-account resources |

## Waves

| Wave | Providers | Hosted tools | Source defs | What the port needs |
| --- | ---: | ---: | ---: | --- |
| 01 Search pilot (Exa, Firecrawl, TinyFish) | 3 | 18 | 12 | Overlays + platform key; Firecrawl overlay already exists in `backend/specs/catalog/` |
| 02 Source-backed data | 16 | 557 | 554 | Native contract review, transformations, pricing evidence |
| 03 Data jobs (Apify, Clay, Cloro, Orbit, Ploid) | 5 | 75 | 77 | Owned start/poll/cancel/result |
| 04 Generation | 9 | 59 | 39 | Reconcile with existing OpenAI, ElevenLabs, Google AI overlays; artifacts and usage pricing |
| 05 Owned services (mail, phones, storage, browsers, machines) | 5 | 79 | 17 | Ownership and lifecycle contracts before any publication |
| 06 Additional native contracts | 49 | 1,652 | 0 | Find executable upstream specs and API operators first |
| Total | 87 | 2,440 | 699 | |

## Already in NyxID

Overlays exist for Firecrawl, OpenAI, ElevenLabs, Google AI, Anthropic, Cohere, DeepSeek, Mistral, OpenRouter, and social/workspace providers. Those rows are connection-category (user key). A NyxID-provided variant is a second catalog service sharing the same spec key with `service_category = internal`, a stored master credential, and `platform_key` enabled.

## Not ported by definition

- Monid's `/v1/run` execution, run ownership, wallet, and ranking: replaced by NyxID proxy, grants, billing, and `nyx__search_tools`.
- Monid's own pricing: user prices are authored in `ServiceBilling` lanes.
- Hosted-only input schemas behind `/v1/inspect`: describe Monid's interface, not the upstream API; used as a hint only.

## NyxID-managed services as Tools

Tools are not limited to imported Monid providers. Any catalog service whose call is answered by a NyxID-held credential is a Tool candidate. Two existing populations qualify today:

- The 31 seeded `internal` catalog rows that run on a stored master credential (`requires_user_credential = false`), including the platform-key rows such as `chrono-llm` and seeded xAI.
- Providers where NyxID already holds app-level credentials for another feature. X is the clear case: the channel adapter stores an `app_bearer_token` in Platform Credentials, and the X overlay's public reads (`search_recent_tweets`, `get_user_by_username`, `get_user_tweets`) work on app-only auth.

The split is per operation, not per provider. For X: public reads become `tools-x` (internal, platform key, app bearer, only the three read operations active, `cost_class = metered` because X usage credits are shared across NyxID's app). `get_me`, `create_tweet`, `delete_tweet`, and the DM operations stay on `api-twitter` as an AI Service under the user's OAuth connection. Both rows share the `twitter` spec key; cards read "Your account" versus "Provided by NyxID".
