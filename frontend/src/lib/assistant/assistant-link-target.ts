import { isHostedConnectLink } from "@/lib/assistant/hosted-connect-link";

export type AssistantModalLinkTarget =
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
