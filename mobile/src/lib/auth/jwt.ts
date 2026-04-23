function normalizeBase64Url(value: string): string {
  const normalized = value.replace(/-/g, "+").replace(/_/g, "/");
  const padding = normalized.length % 4;
  if (padding === 0) return normalized;
  return normalized + "=".repeat(4 - padding);
}

function decodeJwtPayload(accessToken: string): Record<string, unknown> | undefined {
  const payloadSection = accessToken.split(".")[1];
  if (!payloadSection) return undefined;
  if (typeof globalThis.atob !== "function") return undefined;

  try {
    const decoded = globalThis.atob(normalizeBase64Url(payloadSection));
    return JSON.parse(decoded) as Record<string, unknown>;
  } catch {
    return undefined;
  }
}

export function decodeJwtSub(accessToken: string): string | undefined {
  const payload = decodeJwtPayload(accessToken);
  const sub = payload?.sub;
  return typeof sub === "string" && sub.length > 0 ? sub : undefined;
}
