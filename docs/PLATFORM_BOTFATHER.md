# Platform BotFather

A shared bot serves approved public FAQ answers to visitors. A person can then
sign in to NyxID, review consent, and prove control of that exact private channel
conversation to use their existing personal NyxBot. Each platform, bot, sender,
and private conversation needs its own proof. Signing in alone does not link a
messaging identity.

Public FAQ is deterministic: it selects at most two articles from
`backend/content/platform-public-faq.json`. It uses no language model, private
context, personal credentials, or tools. Basic setup needs no sponsor
organization, model discovery, inference budget, or public AI readiness probe.
The older public NyxAgent contract remains unavailable; selecting its old policy
does not make a bot ready. Replace that policy with built-in FAQ in the runtime
controls.

## Set up a shared bot

1. Connect an eligible messaging bot through **Channel bots**. Complete the
   provider-specific connection setup and deliver a private test message so the
   connection becomes active.
2. As a live platform administrator using a first-party human session, open
   **Admin → Platform bots**. Existing shared bots and their Resume actions are
   shown before new setup. Select an eligible bot and choose **Prepare shared
   bot**. Read-only organization bots and already reserved identities are not
   offered. The bot's existing transport owner remains the billing owner.
3. Choose **Public FAQ**, **Personal NyxBot linking**, or both, then **Enable
   bot**. The newly prepared profile remains visible even when it is outside the
   current list page. For Discord, configure the application commands in
   **Connection setup and recovery** first.
4. Test public help, then complete the personal flow below with a test account.
   Public help remains available through its explicit command when a previously
   linked person uses a FAQ-only bot. Ordinary personal text never silently
   changes to another person's authority.

Preparation reserves the exact provider account permanently for shared routing.
It revokes obsolete owner route credentials and queues exact old-work cleanup.
Pause, retirement, deleted local bot rows, and loss of a bot marker do not restore
ordinary channel-owner routing. Reusing the same remote ID on a different
platform is a separate identity. Native Telegram managers remain supported as
`platform=telegram`, `credential_source=telegram_manager`; `telegram-new` normally
identifies a managed child bot and is not an additional generic shared target.

## Provider support and controls

| Provider | Authenticated private input | User controls and setup |
| --- | --- | --- |
| Telegram bot token | Verified bot webhook, original private human messages | `/faq [topic]`, `/help`, `/start`, `/link`, `/confirm P-…`, `/unlink`; ordinary text after linking. The shared send path uses plain text, protected content and no link previews. |
| Native Telegram manager | Existing manager webhook and observation | Same shared controls. Optional native creation/onboarding remains separate from FAQ/personal work and obeys emergency Pause. |
| Slack | Signed JSON Events API `message.im` events | Send exact plain text `help`, `faq [topic]`, `link`, `confirm P-…`, `unlink` in the bot DM. Subscribe to `message.im`, grant `im:history` and `chat:write`, enable App Home messages and reinstall after scope changes. These are ordinary DM messages, not Slack slash commands. |
| Discord | Signed application-command interactions in bot DMs | Use `/faq topic`, `/help`, `/start`, `/link`, `/confirm code`, `/unlink`, and `/chat message` for personal text and action decisions. Set the Interactions endpoint and public verification key in the provider portal, and install with `bot` + `applications.commands`. Ordinary gateway DMs do not reach this webhook. |
| Lark / Feishu | Verified/encrypted message event from a user in a private chat | Configure the callback, verification/encryption fields and bot message permissions shown by connection setup. Use the slash-prefixed shared controls and ordinary private text. |
| WhatsApp | Verified platform webhook with exact sender account | Configure the business phone ID and provider verification credentials. Use the slash-prefixed shared controls in a private conversation. Provider conversation-window and account policies still apply. |
| X | Authenticated private DM webhook or supported existing polling connection | Connect the bot owner's X account through NyxID OAuth. Use slash-prefixed controls and private text. Public posts, mentions and encrypted Chat notifications do not establish personal DM proof. |

Aurinko email is excluded: a claimed `From` address is not equivalent proof of
private account control. OpenClaw is excluded because its current adapter cannot
send the required replies. Other ordinary channel-relay capabilities do not
imply shared FAQ/personal eligibility.

Discord's in-app **Configure Discord commands** action upserts only the known
commands, preserving unrelated application commands. It declares bot-DM context
and exact string options: `confirm.code` required, `chat.message` required, and
`faq.topic` optional. The action fences the current profile and credentials.
Repair clears command readiness; configure again before enabling. `/chat`
argument text cannot execute link, confirmation, unlink, or explicit FAQ controls.

X retains its existing OAuth scopes, webhook subscriptions, billing and polling
policies. Billing-enabled X needs the configured shared app webhook and the
normal channel billing prerequisites; it never silently falls back to paid DM
polling. Existing unbilled polling requires the polling worker and has the
adapter's minimum 60-second interval. X API plan, DM permissions, paid transport
rates, quotas and subscription recovery remain provider constraints. See
[Channel Bot Relay](CHANNEL_BOT_RELAY.md#x-accounts-dms-mentions-and-replies) for
exact configuration and billing rules. These X requirements are not public
inference requirements and do not apply to other providers.

## Personal linking

1. In the private bot conversation, request a link (`link` on Slack, `/link`
   elsewhere). Open the one-time NyxID link from the delivered message.
2. Sign in to the intended personal account. The review identifies the provider,
   bot and private sender/conversation. Review the account, existing link and
   personal access mode, then explicitly consent. Replacing an existing account
   requires a separate acknowledgment.
3. The website reveals a fresh confirmation command only after consent. Send it
   in the same private conversation: `confirm P-…` on Slack, `/confirm code:P-…`
   using Discord's application command, or `/confirm P-…` on other providers.
4. Wait for same-channel confirmation. Use `/chat message` on Discord; send normal
   text on other providers. The linked channel thread appears under **Linked
   channels** and in the assistant sidebar. It uses that person's existing NyxBot,
   permissions, memory, access and personal billing through a separate saved
   channel thread. The existing home transcript is not automatically replayed
   into that thread. Provider transport costs remain with the bot owner.

The link intent lasts up to 24 hours; web consent produces a fresh challenge
valid for at most 10 minutes and no longer than its parent intent. Expired,
wrong-conversation, cancelled, replaced or revoked proof cannot link. Request a
fresh link in the original private conversation and repeat consent. Forwarded
messages may be read as ordinary text but never prove control. Bot, self, edit,
group and malformed provenance cannot enter personal execution. Unsupported human
media receives supported-input help and never becomes personal model input.

The browser scrubs the link fragment and keeps bearer/proof material only for the
active flow. Reload does not reconstruct the confirmation command. Terminal UI
retains only safe provider/bot display metadata; logout/account changes clear the
flow. Repeated confirmed proof revalidates current binding, owner, modes and
manager epoch, so unlink, account disable and repair cannot be undone by replay.

Use `unlink` on Slack or `/unlink` elsewhere to disconnect, or disconnect from
**NyxBot settings → Linked channels**. First-person action decisions use the
server-authored `yes CODE` / `no CODE` notice; on Discord send that message through
`/chat`. Files, long replies and complex action review remain available in the
exact NyxID conversation.

## Pause, resume, repair and retire

**Pause bot** atomically pauses public FAQ and new personal work, including
legacy optional setup entry points. Safe status notices and disconnect
acknowledgments can still be delivered. It preserves the last selected modes.
**Resume bot** restores those selections in one conditional update, including
after a connection repair or missing Discord commands blocked setup. It does
not silently turn a personal-only bot into a public bot.
If both modes were off before Pause, both remain unchecked. Select at least one
mode to enable or resume the bot; the page displays that requirement inline.

Status distinguishes active, paused, setup incomplete, connection repair
required and retired. FAQ readiness is separate from disabled AI readiness; no
AI evidence is fabricated. Missing/revoked/expired OAuth, insufficient scopes,
a disabled provider or credential drift produces repair status while keeping the
recovery controls and safe historical Linked channels inventory accessible.
Database/internal failures remain errors.

Any authorized platform admin can use **Connection setup and recovery** on this
page; the ordinary owner-scoped channel page is not the recovery authority.
**Verify connection** checks this exact provider identity and current connection;
it does not pretend a manual callback is live. **Repair connection** can adopt
replacement credentials only for the same provider account and original owner.
For OAuth, the owner first reconnects the account; the admin may then supply its
owner-scoped connection UUID. Managed platform credentials are changed through
Admin platform credentials. Secrets clear after success or failure and are not
stored in browser storage.

Repair pauses the bot, increments its authority epoch, invalidates old links and
pending replies, and changes profile generation. Old links show disconnected;
everyone needs fresh consent and private proof. Complete the provider callback
setup, send a private test message (or use the provider dashboard test), then
**Refresh status**. Configure Discord commands when required, select the desired
modes and Resume. Old cleanup is bounded and fenced to obsolete work; it cannot
stop a new link's turn after resume. An ownership/account replacement requires
restoring the original identity or retiring the profile and preparing another
eligible bot.

Retirement disables shared work and retains identity tombstones. It is not an
escape hatch to restore ordinary owner routing. Existing history remains subject
to its ordinary owner access and retention rules.

## Delivery, storage and rollout

Admission records an immutable audience. Duplicate provider events cannot change
that audience. Built-in FAQ admission, approved article IDs, queued delivery and
receipt settlement commit together: a crash before the transport call leaves
metadata-only work for the sweep, without retaining the raw FAQ question.
Deterministic event keys and durable exact peer identities do not expire into
new replay authority, and there is no lifetime visitor-count exhaustion gate.
Rate windows and bounded input/work limit ingress.

Each send revalidates live authority and commits its claim with the exact
encrypted bot/platform/OAuth credential snapshot. It sends using only that
captured material, never credentials resolved after the claim. Partial or
uncertain provider effects retain their evidence and are never automatically
resent. A crash after claiming but before an observed provider outcome therefore
requires inspection; exactly-once delivery across a provider boundary is not
claimed.

Discord's deferred interaction context is encrypted, bound to the profile/event
and expires after 14 minutes. Prompt replies complete the original interaction.
A long-running result after that expiry uses the bot token to send to the same
bound private DM; it never reuses an expired interaction token. The provider may
leave its old deferred interaction unresolved. Replies obey Discord's 2,000 UTF-16
unit limit; longer personal output links to the exact NyxID conversation. Shared
text payloads suppress unintended markup, mentions and link previews where the
provider supports those flags.

Upgrade all ingress, execution and delivery replicas before enabling generic
profiles or new runtime/delivery variants. Do not roll back to binaries that
ignore the new authority reservations or platform origins; pause first and retain
tombstones and proof metadata. No production deployment or provider acceptance is
implied by local tests. Use isolated Mongo replica-set tests, mocked provider
requests and browser fixtures, then separately validate real provider setup with
authorized test accounts before production enablement.
