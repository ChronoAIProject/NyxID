# AI Services billing labels

Reviewed with Claude Opus 5.5 at xhigh on 5 October 2026, before implementation.

The label answers two questions in order: does this service have NyxID billing
configured, and who supplied the selected connection credential? Ownership of the
connection (personal, organization or platform) is a separate fact.

Product clarification: **free credit grants, allowances, promotions and wallet
funding never decide this label**. A service configured as billable by NyxID uses
platform billing unless the selected key or developer app is confirmed to be
supplied by the user or organization. `NyxID` is the compact card label for NyxID
platform billing. Grant-funded usage remains NyxID; a supplied key remains BYOK
even when a grant covers an additional NyxID fee. The amount charged, including
a zero wallet debit, cannot change either classification.

| Service billing | Selected credential supplier | Label |
| --- | --- | --- |
| No configured usage charges | Any | — |
| Configured | NyxID key or NyxID OAuth developer app | NyxID |
| Configured | No provider credential required | NyxID |
| Configured | Confirmed person/organization key or developer app | BYOK |
| Configured | Unknown or unresolved supplier | Unverified |
| Configuration unavailable/restricted | Any | Unverified |

The dash tooltip is exactly **Not billable by NyxID**. A provider may charge
separately. All connection categories appear in grouped cards; disabled connections
retain their configured category. A no-auth connection on a service with billing
configured is also NyxID; an absent connection price is separate from the label.

## Backend evidence

- `billing.service_billing_configured`: any configured charge across credential
  classes, independent of caller rollout, health or wallet funding.
- `billing.credential_supplier`: `nyxid`, `own`, `none` or `unknown` for the selected
  context. Restricted results omit it. Agent overrides use the override's metadata.
- `KeyResponse.oauth_app_source`: resolved `platform` or `byo` OAuth app source.
  Both `/keys` and insights use `oauth_app_source::load`: explicit selection,
  an observation from a successful OAuth exchange/refresh, or an exactly matched
  legacy token copy. The lookup never decrypts or returns credentials.
- `UserApiKey.oauth_app_observation` records the actual resolved app source,
  credential epoch and observation time alongside the successful token write.
  A replacement epoch or newer authorization invalidates an old observation.
  A disagreement with explicit selection remains unknown. The observation is
  descriptive metadata and never changes execution's `credential_source`.
- Existing `credit_billing_configured`, `rates` and `charge_status` keep their
  connection-specific meaning and never decide the service-wide label gate.

Explicit platform binding selects NyxID even when an old personal key is retained.
Durable OAuth source wins over retained app hints. A connection ID, stored client
ID or unexpired token alone does not prove which app issued the grant. Older BYO
connections can lack embedded app credentials, and migration could copy an
unrelated app onto a key. Legacy token matching checks owner, provider, token
association and identical copied access/refresh ciphertext; it also handles
migrated rows carrying a connection ID. Missing, changed or ambiguous token
evidence remains unknown. Ciphertext stays inside the resolver. The execution
class `UserOwned` is also the legacy fallback, so it is not
proof of app ownership. For non-OAuth connections, a stored
API-key record follows the supplied-key path; NyxID master credentials are kept
in the catalog and selected by platform binding. A user binding without a stored
key is insufficient. Node routing alone does not establish credential provenance.

## Production-data limits

The current preview can classify unpriced services and supplied keys from existing
APIs. It cannot prove NyxID OAuth app selection until `oauth_app_source` or the
insight supplier field is deployed. Legacy OAuth rows can be resolved from an
exact provider-token copy. Unmarked modern rows acquire verified provenance on
their next successful authorization or refresh; reads do not trigger a refresh.
Missing or ambiguous records still need reconciliation. Missing
private catalog entries cannot be interpreted as absent billing.

Connected-service cards and overview pages request the full accessible catalog,
including internal services omitted by the credential-setup catalog. Insights
cache keys include credential selection, OAuth provenance, owner and connection
pricing metadata, so changes to those inputs cannot retain an earlier label just
because the connection UUID is unchanged. Regression tests cover both cases.

On the latest live read, Chrono LLM is present in both catalog variants with no
billing. These fixes cover reproducible stale/incomplete-data cases; they do not
establish which case produced the previously reported browser label.

Live checks found Twitter with one supplied organization app and three personal
OAuth rows without published app provenance. Two carry a connection ID; the
disabled third uses legacy storage. The owner confirms all three used NyxID's app,
so the expected card, once supported by backend evidence, is **3 NyxID · 1 BYOK**.
Production omits the source field and returns HTTP 404 for `/service-insights`.
Removing client-ID and connection-ID guesses means all four OAuth connections
can show Unverified on that older backend. This is an explicit rollout limit,
not a claim that the user connected them incorrectly. Deployment supplies source
metadata for selected, observed or exactly matched connections; historical rows
without that evidence need a successful refresh or reauthorization.
Available history does not supply that missing source. The legacy token's exact
contents could not be inspected through existing public metadata APIs.
Anthropic, Chrono LLM and Spotify have no billing configuration and show
a dash. DeepSeek has a stored supplied-key connection and platform-only pricing,
so its label is BYOK with no applicable NyxID fee.

## Twitter charging discrepancy

The live Twitter configuration has `platform_billable=true`, a synced platform-key
price of 0.05 credits/request, no BYOK price, and no NyxID-only restriction. Existing
execution deliberately selects the BYOK lane for `NyxidPlatformOauthApp`. A missing
lane is uncharged. Consequently, the catalog platform-key price must not be shown
as the charge for an OAuth connection. This is a configuration/runtime discrepancy
with the intended product behavior, independent of the label correction.

This discrepancy does not make a confirmed NyxID OAuth connection BYOK or
nonbillable in the card. Its service billing is configured and its selected app
is NyxID's, so its label remains NyxID. Rates and settlement must be verified
separately; grants are not part of that classification decision.

This change does not mutate billing configuration or move OAuth into another price
lane. Charging shared-app OAuth could be addressed through an explicit rate and
NyxID-only restriction using the existing lane, or a separately reviewed change to
OAuth lane selection. Either affects money, legacy provenance and agent overrides;
it must not be hidden in a frontend label fix.
