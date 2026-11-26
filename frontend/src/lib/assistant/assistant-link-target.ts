import { ACCOUNT_PANEL_KEYS, parseAccountPanelSearch, type AccountPanel, type AccountPanelParams } from "./account-panel-search";
import { parseRawChannelBotReturnTo } from "@/schemas/channel-bot-setup";
import { isHostedConnectLink } from "@/lib/assistant/hosted-connect-link";
import { isNyxAgentConversationId } from "./conversation-ids";
import { AGENT_DETAILS_TABS } from "./search";

export type AssistantModalLinkTarget =
  | { readonly kind: "account-panel"; readonly href: string; readonly panel: AccountPanel; readonly params: AccountPanelParams }
  | { readonly kind: "agent-key"; readonly href: string }
  | { readonly kind: "channel-connect"; readonly href: string; readonly token: string }
  | { readonly kind: "conversation"; readonly href: string; readonly conversationId: string }
  | { readonly kind: "workspace"; readonly href: string }
  | {
      readonly kind: "connect";
      readonly href: string;
      readonly token: string;
    }
  | {
      readonly kind: "channel-bot";
      readonly href: string;
      readonly platform: string;
      readonly search: string;
      readonly requestId?: string;
    };

const CHANNEL_BOT_SETUP_PATH = /^\/channel-bots\/connect\/([^/]+)$/;
const PLATFORM = /^[A-Za-z0-9][A-Za-z0-9_-]{0,63}$/;
const MAX_SETUP_LABEL_LENGTH = 128;
const SAFE_SETUP_QUERY_KEYS = ["label", "target_org_id"] as const;

function safeChannelSetupSearch(url: URL): string {
  const params = new URLSearchParams();
  for (const key of SAFE_SETUP_QUERY_KEYS) {
    const value = url.searchParams.get(key)?.trim();
    if (value) params.set(key, value.slice(0, MAX_SETUP_LABEL_LENGTH));
  }
  const returnTo = parseRawChannelBotReturnTo(url.search);
  if (returnTo) params.set("return_to", returnTo);
  const serialized = params.toString();
  return serialized ? `?${serialized}` : "";
}

/**
 * Classify links that Nyxbot can complete without leaving the chat surface.
 * The origin check is deliberate: the chat must never turn an arbitrary link
 * into an authenticated in-app modal.
 */
export function assistantModalLinkTarget(
  href: string,
  origin: string,
): AssistantModalLinkTarget | null {
  let url: URL;
  try {
    url = new URL(href, origin);
  } catch {
    return null;
  }
  if (url.origin !== origin || url.username || url.password || url.hash) {
    return null;
  }

  if (isHostedConnectLink(url.href, origin)) {
    return {
      kind: "connect",
      href: url.href,
      token: url.pathname.slice("/connect/".length),
    };
  }

  if (url.pathname === "/channel-bots" && url.searchParams.get("connect") === "telegram-new") {
    const keys = [...url.searchParams.keys()];
    if (new Set(keys).size !== keys.length || keys.some((key) => !["connect", "request_id", "label", "target_org_id"].includes(key))) return null;
    const requestId = url.searchParams.get("request_id") ?? undefined;
    if (requestId && !/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(requestId)) return null;
    return { kind: "channel-bot", href: url.href, platform: "telegram-new", search: safeChannelSetupSearch(url), requestId };
  }

  const workspaceKeys = url.pathname === "/assistant/automations" ? ["setup", "agent"]
    : url.pathname === "/assistant/machines" ? ["tab"] : undefined;
  if (workspaceKeys && [...url.searchParams.keys()].every((key) => workspaceKeys.includes(key)) &&
      new Set(url.searchParams.keys()).size === url.searchParams.size &&
      [...url.searchParams.values()].every((value) => /^[A-Za-z0-9_-]{1,256}$/.test(value))) {
    return { kind: "workspace", href: url.href };
  }

  // Advanced management screens already own their forms and authorization.
  // Keep their canonical entry points in the app without inventing dialogs.
  if (!url.search && (["/assistant/plugins", "/assistant/approvals", "/channel-bots", "/nodes", "/approvals/settings", "/approvals/history", "/approvals/grants", "/orgs", "/triggers", "/developer/apps", "/settings/devices/onboard", "/ai-setup"].includes(url.pathname) || /^\/orgs\/[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(url.pathname))) {
    return { kind: "workspace", href: url.href };
  }
  if (url.pathname === "/keys" && url.searchParams.size === 1 && ["services", "pools", "nyxid"].includes(url.searchParams.get("tab") ?? "")) {
    return { kind: "workspace", href: url.href };
  }

  // Hosted channel setup has its own token and lifecycle; preserve it intact.
  const channelConnect = /^\/connect\/bot\/(nyx_bcl_[A-Za-z0-9_-]{1,152})$/.exec(url.pathname);
  if (channelConnect && !url.search) {
    return { kind: "channel-connect", href: url.href, token: channelConnect[1]! };
  }
  if (url.pathname === "/keys" && url.searchParams.get("tab") === "nyxid" &&
      url.searchParams.get("action") === "create-key" &&
      url.searchParams.size === 2) {
    return { kind: "agent-key", href: url.href };
  }

  if (url.pathname === "/assistant") {
    if (url.searchParams.size === 2 && url.searchParams.has("agent") && url.searchParams.has("details") &&
        /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(url.searchParams.get("agent") ?? "") &&
        AGENT_DETAILS_TABS.some((tab) => tab === url.searchParams.get("details"))) {
      return { kind: "workspace", href: url.href };
    }
    const conversationId = url.searchParams.get("c");
    if (url.searchParams.size === 1 && conversationId && isNyxAgentConversationId(conversationId)) {
      return { kind: "conversation", href: url.href, conversationId };
    }
    if (Array.from(url.searchParams.keys()).some((key) => !ACCOUNT_PANEL_KEYS.some((allowed) => allowed === key))) return null;
    const raw: Record<string, unknown> = Object.fromEntries(url.searchParams);
    if (url.searchParams.has("panelServices")) {
      try { raw.panelServices = JSON.parse(url.searchParams.get("panelServices") ?? ""); } catch { return null; }
    }
    const { panel, ...params } = parseAccountPanelSearch(raw);
    return panel ? { kind: "account-panel", href: url.href, panel, params } : null;
  }

  const match = CHANNEL_BOT_SETUP_PATH.exec(url.pathname);
  if (!match) return null;
  let platform: string;
  try {
    platform = decodeURIComponent(match[1] ?? "");
  } catch {
    return null;
  }
  if (!PLATFORM.test(platform)) return null;
  const search = safeChannelSetupSearch(url);
  return {
    kind: "channel-bot",
    href: `${url.origin}${url.pathname}${search}`,
    platform,
    search,
  };
}
