# Production metadata used by the mockup

The snapshot was read from production through the signed-in NyxID CLI on
2026-09-14. It is stored in `nyxid-service-snapshot.json` and embedded in the
standalone HTML so the mockup also works when opened as a local file.

| Source                                                             | Coverage                                                                                             |
| ------------------------------------------------------------------ | ---------------------------------------------------------------------------------------------------- |
| `nyxid catalog list --all --output json`                           | All 46 returned catalog entries.                                                                     |
| `nyxid catalog show <slug> --output json`, for every returned slug | Full published scope catalogs, capability metadata and documented setup requirements.                |
| `nyxid service list --output json`                                 | All 39 returned connections, including personal, organization, custom, disabled and expired records. |
| `nyxid api-key list --output json`                                 | Rechecked: no personal Agent Keys returned.                                                          |

The catalog contains 171 published scope entries across 18 service catalogs
(127 distinct raw scope strings). Definitions can share a scope string across
providers. The picker preserves service-qualified identities instead of merging
their labels or descriptions. It also includes scopes recorded on connections
but absent from their provider's published menu. This covers the full metadata
NyxID publishes in these responses, rather than claiming a complete list of
every permission an upstream provider could ever accept.

Examples include 21 Twitter/X permissions, 13 GitHub permissions, 12 each for
Lark and Feishu, 11 each for Microsoft Graph and Spotify, 10 for Slack, and
9 each for Discord and Reddit. The original eight-connection sample has been
replaced by the complete returned catalog and connection inventory.

Connection cards use actual `granted_scopes`. Catalog cards use `scope_catalog`
and label it as available access. Required setup permissions and service
capabilities appear separately; neither proves that a credential holds a scope.
Google userinfo email/profile scopes use the equivalent labels in the same
Google service catalog; exact raw identifiers remain available in the disclosure.
The picker deduplicates these Google aliases and uses equivalent spelling for
matching. Other providers retain their own scope identities and descriptions.

The permission recheck fetched all 46 service details again: all 171 published
entries matched the previous snapshot, including scope identifiers, labels,
descriptions and flags. The 39 connection permission sets and credential/service
states also matched; two arrays only changed ordering. The refreshed snapshot
now includes `platform_scope_allowlist`. GitHub `write:org` and `delete_repo`,
and five scope choices on the generic Google API service, are outside the
shared-app allowlist and are labelled accordingly in the picker.

This verifies agreement with NyxID production data on 2026-09-14. The backend's
`services/scope_catalog.rs` explicitly describes its catalog as curated,
non-exhaustive and subject to provider consent. It does not independently prove
every upstream description, account entitlement, or acceptance of every
combination of scopes. Real connection cards display recorded grants;
catalog menus describe permission options rather than authenticated authority.

Only names, slugs, service UUIDs, ownership labels, credential type/status and
permission/capability metadata are included. No credentials, key IDs, endpoint
URLs, account email addresses, OAuth client credentials or tokens are embedded.

Organization service visibility does not establish permission to administer its
Agent Keys. The current account's organization key listing was previously denied
with 403. These connections remain visible and cannot be selected for a personal
key in the preview. The default preview has no personal Agent Keys, matching
the response. Node data was not loaded.

A separate `preview_keys=sample` design example adds three labelled sample Agent
Keys: Mail & code assistant, Mail & code with sending, and Workspace developer.
Their key names, key IDs, API scope assignments and expiry settings are design
fixtures. Their connections and provider permissions use the real snapshot:
Gmail read/identity plus GitHub repo/profile/email, a Gmail connection with send
access, and the base connections with additional NyxID write access. The page
and each card disclose sample keys; **Use actual snapshot** removes them.
These fixtures are not represented as keys minted on the account.

The HTML simulates selection and review. It performs no live key issuance,
approval, connection or provider call. See `permission-filter-contract.md` for
the proposed URL format and filtering behavior.

## Service icons

The standalone mockup clones static HTML templates rendered from the existing
`frontend/src/components/service-icons/index.tsx` registry: all 38 service
glyphs plus its generic globe fallback. Composite glyphs retain the app's
provider mark and function badge. The NyxID group uses the existing public
NyxID icon. Icons appear in selected-permission group headings, dropdown group
headings, matching-key permission rows, service connection cards and selected
connection chips. Unknown catalog services use the same fallback as the app;
no external logo service is contacted. Service names remain visible alongside
icons, and full permission names remain accessible.
