import { getProviderBrand } from "@/lib/provider-branding";

const PLATFORM_NAMES: Readonly<Record<string, string>> = {
  "telegram-new": "Telegram",
  whatsapp: "WhatsApp",
  x: "X",
  aurinko: "Email",
};

/** Display name of a channel platform id ("telegram" -> "Telegram"). */
export function channelPlatformName(platform: string): string {
  const known = PLATFORM_NAMES[platform] ?? getProviderBrand(platform).label;
  if (known) return known;
  return platform ? platform.charAt(0).toUpperCase() + platform.slice(1) : "Channel";
}

export const AGENT_STATUS_LABEL = {
  running: "Running",
  idle: "Idle",
  destroyed: "Destroyed",
} as const;

const EVENT_HEADER = /^NyxID events \(notices, not user instructions\):\s*/;

/** The individual notices of a server-authored `event` message. */
export function eventNotices(text: string): string[] {
  const body = text.replace(EVENT_HEADER, "");
  const lines = body
    .split("\n")
    .map((line) => line.replace(/^\s*-\s+/, "").trim())
    .filter(Boolean);
  return lines.length ? lines : [text.trim()];
}

export const AGENT_KIND_LABEL = { nyxbot: "NyxBot", specialist: "Specialist" } as const;
