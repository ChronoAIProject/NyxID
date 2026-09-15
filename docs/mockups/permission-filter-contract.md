# Device login permission filters

The approved visual reference is `login-consolidated.html`. The actual React
implementation is `frontend/src/components/auth/device-approval.tsx`, with
`login-permission-picker.tsx` and `login-key-draft.tsx`. Production uses live
options and catalog APIs, not the HTML's sample accounts or preview inventory.
The complete URL and approval contract is [DEVICE_LOGIN_PROTOCOL.md](../DEVICE_LOGIN_PROTOCOL.md).

## Picker

Search is a standalone single-line input. Selected filters appear below in a
separate panel grouped by service icon/name, with overall and group counts and
removable pills. Search matches service names, catalog labels/descriptions and
raw scopes. Exact scope strings remain available in tooltips. No label invents
provider grants. Group “All N permissions” selects the full current group even
when search hides some choices; partial selection has a mixed state. It does
not grant future permissions. Unhighlighted Enter never toggles the first item.
Escape closes the menu and returns focus; pill removal stays in the selection
panel. Selecting filters does not approve or change provider credentials.

When creating a key, the same dropdown includes usable connections under each
service. Search matches permission labels, service names, and connection labels
or slugs. Connection choices show their actual permissions and match status.
Catalog services without a connection are marked “Not connected”; their permission
filters remain available. Selected connections appear in compact cards below the
requested filters. There is no separate service-selection list. A key that needs
only NyxID account permissions can be created without selecting a service.

The create path is labelled **New Agent Key · Draft**. **Services this key can
use** lists existing connected accounts in a bounded card, with an **Add service**
action that clears and focuses the same dropdown. Each card separates requested
permissions from extra permissions included. Selecting a connection edits the
draft; **Create & continue** creates one Agent Key and approves the requester.
It does not create external service accounts. The final preview names access
outside the filters **Access beyond the requested filters**.

Existing-key approval and new-key creation use the same final access-review
component. The only difference is the outcome badge: **Existing key** means the
selected key is reused, while **New form · creates key** means one new Agent Key
will be created when the approval action is submitted.

## Hints and matching

`permissions` contains supported NyxID API scopes; `services` contains catalog
slugs; `service_permissions` contains `slug::scope` or a bare scope filter.
Repeated/CSV values are supported within the parser's documented limits. Other
draft hints are `login_type`, `key_source`, `key_name`, `expiry_days`, `platform`.
Unknown, malformed or duplicated singleton parameters fail visibly. Unknown
services/scopes remain unresolved until removed or corrected. All values are
editable requester suggestions; ownership, IDs, wildcard grants, snapshots,
capabilities and credentials cannot come from URL hints.

Existing keys must cover every requested service/permission with usable proxy
access. The server's effective binding summary is authoritative. Exact matches
come first, followed by matches with visible extras: identity scopes, additional
services/nodes, unknown provider access and current/future grants. An unavailable
extra does not invalidate healthy requested access. An unavailable requested
connection cannot match. Refreshable OAuth access-token expiry alone does not
make a connection unavailable.

Platform services may have no user credential. Availability follows the existing
platform binding/availability policy; provider access remains unreported. Durable
`allow_auto_connected_services` includes current/future active platform rows
owned by the key owner, and is always extra access. Other owners' rows are not
implied. Eligible personal/org explicit selections follow backend scope policy.

## Creation and approval

The requested filters and actual new-key grant remain separate. Exactly one
eligible exact connection per service may prefill a draft; broader or ambiguous
connections require explicit selection. Detailed settings are collapsed; owner
changes reset resource and wildcard grants. Reuse the platform scope control
for individual platform rows or its explicit durable grant. Show all actual
extras, including implied platform rows, before “Create & continue”.

The final action creates/approves through the backend. Existing keys submit their
permission snapshot; new keys submit snapshots for explicit and current implied
connections. Changes require refresh and review. Scanning, selecting filters,
request acknowledgement and signing in never approve. Denial is terminal.
