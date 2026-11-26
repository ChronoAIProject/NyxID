# Channel bot guest setup

## Inventory a pilot before enabling it

Existing bots default to ordinary guest behavior. New bots save the guest
access setting as on, including a second bot created by an existing owner.
The global `nyxbot:guest-audience-v1` flag and the platform's rollout selection
determine when the bot uses that setting. Global off restores ordinary behavior
for fresh messages across all bots. During a named pilot, only selected bots
with the setting on can use the new rules. For an external route, setup must
also assign a dedicated, compatible key.

For a single-bot test, leave the global flag off while a platform admin saves
the pilot with `PUT /api/v1/admin/feature-flags/nyxbot-guest-audience/pilot`
and the body `{"bot_ids":["<test-bot-id>"],"fleet":false}`. Read the selection
back with `GET` on the same endpoint. In the test bot's settings, check
**Use guest access v1** and prepare any required data grants and pin audiences.
Once the serving replicas are ready, enable the global flag. An empty pilot
selects zero bots. Turning off the test bot restores ordinary handling for its
fresh requests. Turning off the flag does the same across all bots. Both actions
invalidate affected guest work already in flight.

Use the explicit bot IDs from the proposed pilot selection. Run the inventory from the repository root against the NyxID database with a read-only MongoDB account. The script accepts up to 100 bot IDs and returns JSON.

```sh
export NYXID_PILOT_BOT_IDS='11111111-1111-4111-8111-111111111111,22222222-2222-4222-8222-222222222222'
mongosh "$NYXID_PILOT_DATABASE_URI" --norc --quiet \
  --file scripts/channel-guest-pilot-inventory.js > channel-guest-pilot-inventory.json
```

- In `hosted_bindings`, review the default agent and any per-chat agent overrides. Compare enabled pins, dependency versions and digests, existing skill audience choices, and ordinary guest service levels and operation selections. An empty operation selection for an applicable scope can withhold the service entirely. Record which existing guest workflows need an explicit data grant and which old skill contents can inform guest answers.
- In `external_routes`, review each active route and its assigned key. The report includes callback presence, existing key access breadth, reuse on another bot or device route, and conflicts with hosted or pending assignments. A dedicated key candidate passes this metadata check. Owners still need to test callback delivery and any separately held provider credentials.
- Use `owner_notice_recipient_ids` to address the copy below. It identifies the active personal owner or the organization's active administrators. An empty recipient list needs resolution before that bot enters the pilot.

The inventory reads projected configuration metadata and emits account, bot, route, key and service identifiers. It excludes credential values and hashes, callback URLs, provider sender IDs, chat content, and skill files. A failed or oversized query stops the report. Use a smaller cohort when the per-query limit is reached.

On a bot selected for the channel guest rollout, open **Guest experience** when you finish connecting it or from the bot details page. NyxBot can give the owner the same settings link. A personal bot owner or an administering organization member uses their first-party NyxID session to edit the configuration.

1. Choose **Owner only**, **Selected verified guests**, or **Guests allowed by the channel settings**. The last choice uses the bot's existing per-channel admission settings. A selected guest still needs channel admission. Verified organization members retain their own authority.
2. Add an explicit data rule with an audience, source, actions and, where applicable, fields. A new or enrolled bot with no applied configuration has zero guest data grants. **Save draft** preserves choices while you connect services or verify guests. The currently applied policy governs execution until you apply an update.
3. Use **Preview two guests** to review access across two identities, including cross-guest access to an example record and whether the same information could be used in a group. The server runs the same data-policy evaluator with sample records. Apply once the result matches the intended policy. Preview results describe configuration decisions. Confirm transport delivery, service authorization and independent approvals separately in an end-to-end test.

### Prepare a policy before enrollment

Open **Prepare guest access** in the bot settings. The staging editor keeps the bot on its ordinary settings while you save a draft, review any existing operations without a replacement, preview two guests, and mark the saved draft for activation. Named guest and room selection requires verified channel observations for this installation. The owner can enable **Use guest access v1** before or after preparing the draft. A marked draft is applied once the bot is selected and its authorization still matches. If the owner, pin, or grants change, review and mark a corrected draft to retry.

For an external route, open **Route guest access** in the route's actions. Choose a compatible key and dedicate it to the exact route. The create dialog also supports dedication. A newly configured route continues ordinary delivery until its key qualifies, and the route status shows whether a dedicated key or completion of a legacy callback is required.

## Available information sources

**Personal records** is a bot-owned collection with Title and Body fields. The **Manage their own data** preset grants create, read, update and delete on each verified guest's own records in private conversations. Each action and its supported fields can be selected independently. Record ownership comes from the authenticated channel account. A guest who changes chat accounts has a different identity.

**Google Sheets** appears when an eligible hosted agent has a connected Sheets or Workspace service and the ordinary guest operation ceilings allow read or append. A read grant names one connection, workbook and bounded A1 range such as `Event_Config!A1:H24`. A separate append grant names one workbook and one entire tab such as `Questions` and applies to shared channel conversations. The tool appends one row and Google determines its placement within that tab. Existing service, operation and approval requirements also apply. Use Personal records for own-record updates or deletions.

## Attached skills

A pinned skill and its supporting files can inform guest answers. Review their contents and the selected private and shared audiences before exposing the attachment. Maintaining a bot attachment requires bot and agent management authority and existing package authorization. Under All Requests, a standing approval is scoped to the exact attachment and revision. Existing applicable approvals can also authorize the read.

A maintainer can select a pinned skill declaration in the editor. The server reads the verified package's root `nyxid-guest-access.json` using the exact authorized pin. Version 1 contains a `version` value of `1` and a `rules` array of the same rules accepted by the editor API. Applying those rules creates a new policy revision. Preview shows the effective rules before applying. Repinning or changing the bound agent requires a new review of the declaration.

## Owner communication copy

> Review Guest experience for each bot in the pilot. Choose who can use it and which information they can access. A connected service alone grants no guest data access. Review all attached skill files before making them available to guests, since the bot can use their contents in answers. Use Preview two guests to check the configuration, then test delivery on your channel before inviting guests.

If the platform disables the pilot, fresh messages follow each bot's current ordinary settings. Provider records already created during pilot operation remain with their providers. Data that needs protection in both modes needs provider-enforced access controls or a source isolated from ordinary bot credentials.
