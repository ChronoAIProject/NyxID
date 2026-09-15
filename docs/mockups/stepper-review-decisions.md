# Stepper and consent decisions for review

The HTML reference and React approval page implement these decisions. See
[the protocol](../DEVICE_LOGIN_PROTOCOL.md) for the query and approval contract.

## Request acknowledgement and header

Step 1 uses **This is my request — continue**. The button itself acknowledges
that the user started the request and checked the code. There is no agreement
checkbox and no second acknowledgement at approval. Future steps stay locked
until this action; opening a prefilled URL cannot perform it. **Deny request**
ends the request from any step. Closing the page leaves it pending.

The header contains the NyxID logo and one title. The user code, requester,
client, network, location and expiry appear inside step 1. Steps 2 and 3 show
only a compact CLI/code summary with **Review request**. Sample-data disclosure
is preview chrome and is not part of the proposed production header.

## Steps and consent

1. **Verify request:** acknowledge the request with the affirmative CTA.
2. **Choose access:** full account session or restricted Agent Key. A URL may
   preselect this choice; the user still continues explicitly.
3. **Scope & approval:** full account access skips scope configuration and has
   **Approve full account access**. Restricted access begins with requested
   permissions and matching existing keys immediately below.

Completed steps can be revisited without losing configuration. Final approval
is available only in step 3. An existing key uses **Approve access with this
key**. **Create new Agent Key** adds connection choices to the permission dropdown.
Selected connections and their actual permissions appear below it, followed by
the key name and settings. Its final action is **Create & continue**, with copy
stating that this creates the key and approves the device with the shown access. There is
no separate review step or repeated confirmation. Detailed key settings live
in a disclosure; the final grant summary and extra access stay visible.

Creation is labelled **New Agent Key · Draft** before the search. The selected
connections sit in **Services this key can use**, with a count and **Add service**
action that opens the shared dropdown. These are existing accounts whose access
will be included in the key. Their cards split requested and extra permissions;
the final **Key to create** preview lists access beyond the filters. No service
account is created by this page, and no Agent Key is created before the final CTA.

Creation stays disabled while a name, requested API permission or requested
service is missing, or filters/settings are invalid. Filters are editable
suggestions; users can explicitly revise them. They never trim an existing
key's or service connection's authority.

## Matching choices

Exact matches appear first, then matches with extra access, sorted by the
number of additional access entries and then by name. Cards group their full
matched permissions by service. **Also grants — included with this key** lists
the additional permissions. Identity permissions count as extras when absent
from the request. Additional services, nodes, future-resource grants and
unreported provider access prevent an exact-match claim.

Service connection cards in the create path use the same comparison for their
own service. A Gmail connection is not presented as covering a GitHub request.
Catalog templates are available options, not selectable existing grants.

No personal Agent Keys were returned in the production snapshot. The normal
preview shows that empty state. **Preview matching Agent Keys** opens a separate
`preview_keys=sample` example with explicitly labelled sample keys backed by
real connection permissions. Its request includes the recorded identity
permissions, so the exact-match example is honest. Removing one of those
requested identity permissions turns the first key into a match with extras.

## Permission-group controls

A service with named permissions has a stable **All N permissions** parent
checkbox and individual children. Partial selection shows its count and mixed
state. The control does not change its label, search text or list position.
Enter requires a highlighted option. Services without published permissions
use a simple service option. All enumerates the current named permission set;
it is not a wildcard for future permission additions or every connection.

## Validation

Browser interaction checks passed for acknowledgement and stepper guards,
terminal denial, completed-step navigation, full/existing/create approval,
exact-first ordering, visible extra identity/provider/API access, no automatic
selection or grants, URL presets and malformed hints, missing requested scopes
and services, stable group selection/search/scroll, the complete 46-service /
171-permission / 39-connection snapshot, and desktop/mobile layouts.
No real credential or key was issued.
