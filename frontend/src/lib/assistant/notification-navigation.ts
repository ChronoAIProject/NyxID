import { isAssistantShellRoute, type AssistantShellRoute } from "@/lib/assistant/shell-routes";
import type { AssistantNotification } from "@/schemas/assistant-notifications";

/**
 * Where a notification row leads. Server URLs are `AssistantPage` links
 * (`/assistant?c=…`, `/assistant?g=…`, `/assistant/automations?agent=…`);
 * only same-origin assistant routes become in-app navigation, with the
 * query keys those routes own. Anything else stays a plain link.
 */
export type NotificationTarget =
  | {
      readonly kind: "route";
      readonly to: AssistantShellRoute;
      readonly search: Readonly<Record<string, string>>;
    }
  | { readonly kind: "link"; readonly href: string };

const ROUTE_SEARCH_KEYS: Record<AssistantShellRoute, readonly string[]> = {
  "/assistant": ["c", "g", "agent", "card", "details"],
  "/assistant/plugins": ["tab", "agentId"],
  "/assistant/approvals": [],
  "/assistant/automations": ["agent", "setup"],
  "/assistant/machines": ["machine", "tab"],
  "/assistant/machines/new": ["setup"],
  "/assistant/machines/pair": ["code"],
};

export function notificationTargetFromUrl(href: string, origin: string): NotificationTarget | null {
  let url: URL;
  try {
    url = new URL(href, origin);
  } catch {
    return null;
  }
  if (url.username || url.password) return null;
  if (url.origin !== origin) return { kind: "link", href: url.href };
  const path = url.pathname.replace(/\/+$/, "") || "/";
  if (!isAssistantShellRoute(path)) return { kind: "link", href: url.href };
  const search: Record<string, string> = {};
  for (const key of ROUTE_SEARCH_KEYS[path]) {
    const value = url.searchParams.get(key);
    if (value) search[key] = value;
  }
  return { kind: "route", to: path, search };
}

/** The server link first; the row's own source identifiers when it has none. */
export function notificationTarget(
  notification: Pick<AssistantNotification, "url" | "source">,
  origin: string,
): NotificationTarget | null {
  const fromUrl = notification.url ? notificationTargetFromUrl(notification.url, origin) : null;
  if (fromUrl) return fromUrl;
  const { source } = notification;
  if (source.conversation_id) return { kind: "route", to: "/assistant", search: { c: source.conversation_id } };
  // A channel chat is a conversation too: its source id is the thread id.
  if (source.type === "conversation" || source.type === "channel_chat") {
    return { kind: "route", to: "/assistant", search: { c: source.id } };
  }
  if (source.type === "group" || source.type === "collaboration_group") {
    return { kind: "route", to: "/assistant", search: { g: source.id } };
  }
  if (source.type === "trigger") {
    return {
      kind: "route",
      to: "/assistant/automations",
      search: source.agent_id ? { agent: source.agent_id } : {},
    };
  }
  return null;
}
