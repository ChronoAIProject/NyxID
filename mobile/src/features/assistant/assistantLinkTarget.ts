const ACCOUNT_KEYS = new Set([
  "panel",
  "panelTab",
  "panelPeriod",
  "panelService",
  "panelServices",
  "panelAction",
]);
export type AssistantLinkTarget =
  | { kind: "connect"; href: string; token: string }
  | { kind: "channel"; href: string; platform: string }
  | {
      kind: "account";
      href: string;
      panel: "settings" | "billing" | "nyxbot";
      tab?: string;
      period?: "24h" | "7d" | "30d";
    };

/** Only the configured first-party origin can select authenticated native UI. */
export function assistantLinkTarget(
  href: string,
  frontendUrl: string,
): AssistantLinkTarget | null {
  try {
    const origin = new URL(frontendUrl).origin;
    const url = new URL(href, origin);
    if (
      !/^https?:$/.test(url.protocol) ||
      url.origin !== origin ||
      url.username ||
      url.password ||
      url.hash
    )
      return null;
    if (
      /^\/connect\/nyx_clk_[A-Za-z0-9_-]+$/.test(url.pathname) &&
      !url.search
    ) {
      return {
        kind: "connect",
        href: url.href,
        token: url.pathname.slice("/connect/".length),
      };
    }
    const channel = /^\/channel-bots\/connect\/([^/]+)$/.exec(url.pathname);
    if (channel) {
      const platform = decodeURIComponent(channel[1]!);
      if (/^[A-Za-z0-9][A-Za-z0-9_-]{0,63}$/.test(platform))
        return { kind: "channel", href: url.href, platform };
    }
    if (
      url.pathname !== "/assistant" ||
      [...url.searchParams.keys()].some((key) => !ACCOUNT_KEYS.has(key))
    )
      return null;
    const panel = url.searchParams.get("panel");
    if (
      url.searchParams.getAll("panel").length !== 1 ||
      !["settings", "billing", "nyxbot"].includes(panel ?? "")
    )
      return null;
    const period = url.searchParams.get("panelPeriod");
    return {
      kind: "account",
      href: url.href,
      panel: panel as "settings" | "billing" | "nyxbot",
      tab: url.searchParams.get("panelTab") ?? undefined,
      ...(period && ["24h", "7d", "30d"].includes(period)
        ? { period: period as "24h" | "7d" | "30d" }
        : {}),
    };
  } catch {
    return null;
  }
}
export function assistantLinkLabel(
  target: AssistantLinkTarget | null,
  label: string,
): string {
  if (target?.kind === "channel")
    return `Continue on the web${label.trim() && !label.startsWith("http") ? ` · ${label.trim()}` : ""}`;
  if (target?.kind !== "connect") return label;
  const trimmed = label.trim();
  return /^connect\b/i.test(trimmed)
    ? trimmed
    : `Connect ${!trimmed || trimmed.startsWith("http") ? "service" : trimmed}`;
}
