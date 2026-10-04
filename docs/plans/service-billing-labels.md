# AI Services billing labels

Reviewed with Claude Opus 5.5 at xhigh on 5 October 2026, before implementation.

The label answers two questions in order: does this service have NyxID billing
configured, and who supplied the selected connection credential? Ownership of the
connection (personal, organization or platform) is a separate fact.

| Service billing | Selected credential supplier | Label |
| --- | --- | --- |
| No configured usage charges | Any | — |
| Configured | NyxID key or NyxID OAuth developer app | NyxID |
| Configured | Confirmed person/organization key or developer app | BYOK |
| Configured | Unknown or unresolved supplier | Unverified |
| Configuration unavailable/restricted | Any | Unverified |

The dash tooltip is exactly **Not billable by NyxID**. A provider may charge
separately. All connection categories appear in grouped cards; disabled connections
retain their configured category. A no-auth connection with no applicable lane is
unpriced; legacy configuration that charges unauthenticated usage remains NyxID.

## Backend evidence

- `billing.service_billing_configured`: any configured charge across credential
  classes, independent of caller rollout, health or wallet funding.
- `billing.credential_supplier`: `nyxid`, `own`, `none` or `unknown` for the selected
  context. Restricted results omit it. Agent overrides use the override's metadata.
- `KeyResponse.oauth_app_source`: stored `platform` or `byo` OAuth app selection;
  null/absent on legacy rows. It exposes no credential material or developer app ID.
- Existing `credit_billing_configured`, `rates` and `charge_status` keep their
  connection-specific meaning and never decide the service-wide label gate.

Explicit platform binding selects NyxID even when an old personal key is retained.
Durable OAuth source wins over retained app hints. OAuth without a source or known
supplied app stays unverified: the execution class `UserOwned` is also the legacy
fallback, so it is not proof of app ownership. For non-OAuth connections, a stored
API-key record follows the supplied-key path; NyxID master credentials are kept
in the catalog and selected by platform binding. A user binding without a stored
key is insufficient. Node routing alone does not establish credential provenance.

## Production-data limits

The current preview can classify unpriced services and supplied keys from existing
APIs. It cannot prove NyxID OAuth app selection until `oauth_app_source` or the
insight supplier field is deployed. Legacy OAuth rows without durable provenance
will still require evidence from their original authorization records. Missing
private catalog entries cannot be interpreted as absent billing.

Connected-service cards and overview pages request the full accessible catalog,
including internal services omitted by the credential-setup catalog. Insights
cache keys include credential selection, OAuth provenance, owner and connection
pricing metadata, so changes to those inputs cannot retain an earlier label just
because the connection UUID is unchanged. Regression tests cover both cases.

On the latest live read, Chrono LLM is present in both catalog variants with no
billing. These fixes cover reproducible stale/incomplete-data cases; they do not
establish which case produced the previously reported browser label.

Live checks found Twitter with one supplied organization app and three OAuth rows
without published provenance; the truthful current preview is one BYOK and three
Unverified. Anthropic, Chrono LLM and Spotify have no billing configuration and show
a dash. DeepSeek has a stored supplied-key connection and platform-only pricing,
so its label is BYOK with no applicable NyxID fee.

## Twitter charging discrepancy

The live Twitter configuration has `platform_billable=true`, a synced platform-key
price of 0.05 credits/request, no BYOK price, and no NyxID-only restriction. Existing
execution deliberately selects the BYOK lane for `NyxidPlatformOauthApp`. A missing
lane is uncharged. Consequently, the catalog platform-key price must not be shown
as the charge for an OAuth connection. This is a configuration/runtime discrepancy
with the intended product behavior, independent of the label correction.

This change does not mutate billing configuration or move OAuth into another price
lane. Charging shared-app OAuth could be addressed through an explicit rate and
NyxID-only restriction using the existing lane, or a separately reviewed change to
OAuth lane selection. Either affects money, legacy provenance and agent overrides;
it must not be hidden in a frontend label fix.
