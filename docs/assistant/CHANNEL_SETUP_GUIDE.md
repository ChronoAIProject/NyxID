# Channel setup guide, contract 1 (source checked 2026-10-19)

This guide is built into NyxBot. Ornn is optional. Source verification covers NyxID routes, UI and protocol fixtures; it does not establish live provider account eligibility, deployed gateway support or delivery to a real recipient.

## Discover before changing anything

Use list_channel_adapters for phone/SMS/speech profiles and their rollout/readiness limits; use list_channel_bots, get_channel_bot, list_channel_agents and list_channel_chats for messaging bots. Preserve native public/personal platform entry authority: its channel installation is not an ordinary personal bot to relink. Account tools are authorized for the acting person and their administered organizations. A specialist or guest cannot acquire setup authority from this guide.

Keep identifiers distinct: catalog slug selects a service template for connect_service; service connection ID selects an existing user service; connection_id on a channel bot identifies its credential (UserApiKey); bot_id selects a channel bot; channel_agent_id selects the NyxBot link; provider slug selects an authorization provider. Never pass a connection UUID/label as a catalog slug. For Service not found, inspect the actual argument, list catalog/services and use the returned catalog slug. Reuse a suitable existing account instead of deleting it or repeating authorization blindly.

For a new messaging bot use channel_bot_setup_link with the catalog platform. Give the returned URL to the owner; credentials belong on that page. For an existing bot inspect get_channel_bot first. Only then use connect_channel_bot by returned bot ID or label. Use settings_link for supported settings URLs. Tool parameters do not prove that a frontend button exists. An unavailable setup link or unsupported transport is a blocker to report, not a reason to invent a URL or relax authentication.

## X DMs: applicable setup and actual route

Applies to the managed X OAuth user-connection adapter (platform x, provider twitter), not app-only bearer authorization. The channel setup page is the URL returned by channel_bot_setup_link(platform=x). Its managed OAuth flow selects/creates an account connection. Check credential_source, connection_id and current capabilities before suggesting scopes. NyxID's current X adapter requests user scopes according to selected events; DM receiving/sending needs the adapter's DM read/write authorization. App-only authorization does not stand in for user authorization. Provider enrollment, API credits and account DM recipient restrictions remain separate conditions.

For a paid deployment, DMs arrive through X Activity webhooks at POST /api/v1/webhooks/channel/x/platform, authenticated with the configured shared X app secret and tagged subscription nyxid:{bot_id}. NyxID routes through channel_conversations to /api/v1/nyxbot/relay/{channel_id}; the reply uses /api/v1/channel-relay/reply and X POST /2/dm_conversations/{id}/messages. Legacy DM polling is available only with billing disabled. Paid polling fails webhook_required. A gateway-x flag alone proves no X gateway support: if the selected gateway refuses X, retain the working direct route and report the refusal. Gateway diagnostics unavailable means unknown, not healthy.

The owner may explicitly use Verify or Resubscribe in the existing channel-bot UI/CLI after inspecting health. Those operations may change subscription state; a read does not run them. Subscription read-back is timestamped evidence. A connected OAuth account alone proves neither a subscription nor an agent route. Link the actual bot to the intended agent and complete sender verification using its returned link/code. Check per-chat reply_mode/members and private_chats. Other senders retain guest isolation. API read/write success proves neither inbound delivery nor a continuously enabled automatic-reply loop.

## Evidence and Q&A

Report each stage with its evidence timestamp and unknown/blocked state: account configured; authorization usable; subscription/receive path; authenticated inbound; agent route and sender verification; confirmed outbound; end-to-end auto reply. A last successful send is historical evidence, never a guarantee. No inbound evidence means not observed, not proof of a broken parser or subscription. Test messages require a designated recipient and explicit owner authorization.

Why connected but no DMs? Inspect get_channel_bot health and subscription_state; authorization, subscription, provider enrollment/credits and recipient policy are independent. 401/auth_revoked or scopes_missing: owner reconnects the selected account. A 403 must be classified: scopes_missing/access_denied differs from app_access_denied; the latter requires the platform administrator. 402/provider_credits: administrator restores shared X API credits (separate from NyxID billing_suspended). Missing webhook credentials: administrator. Missing subscription: owner explicitly Resubscribes, then checks read-back. 429: wait until the reported reset. Recipient unreachable: the designated recipient must permit DMs. Unknown outbound: do not resend an ambiguous send. Never mark a partial setup complete.

For Telegram groups, /start@ActualBot is the standard directed command; arguments follow a space. Inspect the discovered bot username, authenticated inbound timestamps and list_channel_chats registration evidence. A group can be registered without a generated welcome reply. If no update was received, check platform delivery/privacy conditions without changing privacy or guest policy to force a response. Source fixtures cover entity offsets; a live provider check remains a separate authorized step.

## Optional Ornn distribution

This same guide can be packaged through the existing authored-skill preview/publish workflow and attached by exact version and hash under the owner's identity. Publication and attachment require their existing approvals and live ACLs. No implicit installation, publication or permission changes occur. Built-in discovery above remains available with no skill attached or with Ornn unavailable.

## Quiet delivery and rollout

NyxBot emits no queue acknowledgement or continuing-work notice. A successful
empty answer finishes silently; images and real failure/partial/stopped outcomes
retain their own delivery policy. Gateway quiet support requires verified
`reply.ack_after_ms=0`, including reconciliation of existing channels. This was
source-checked against CMA develop commit
37547e73aebfc8335b38f914f3ff1db4fcbf950d on 2026-10-19; a deployed gateway
refusing that policy is a capability blocker. No live provider test is implied.

Accepted busy questions retain original destinations in NyxBot's bounded queue,
with no latest-message fallback. Each Event turn answers one question; general
async updates retain their ordinary reply channel. Targets must be minted in the
original event window and expire within twenty-four hours. Upgrade and drain all
old queue writers before guaranteeing source retention; mixed old `$slice`
writers can silently evict sources. This queue does not change the ADR-013
generic relay passthrough contract.
