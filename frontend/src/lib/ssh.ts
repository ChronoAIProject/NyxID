export function parseAllowedPrincipals(input?: string): string[] {
  return (input ?? "")
    .split(/[\n,]/)
    .map((principal) => principal.trim())
    .filter(Boolean);
}

export function deriveNyxidBaseUrl(nodeWsUrl?: string): string {
  if (nodeWsUrl) {
    try {
      const parsed = new URL(nodeWsUrl);
      parsed.protocol = parsed.protocol === "wss:" ? "https:" : "http:";
      parsed.pathname = "";
      parsed.search = "";
      parsed.hash = "";
      return parsed.toString().replace(/\/$/, "");
    } catch {
      // Fall through to the browser origin.
    }
  }

  if (typeof window !== "undefined") {
    return window.location.origin;
  }

  return "";
}
