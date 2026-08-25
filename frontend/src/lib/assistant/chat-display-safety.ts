const SECRET_VALUE_PATTERN =
  /((?:Bearer|Basic)\s+)[A-Za-z0-9._~+/=-]+|\beyJ[A-Za-z0-9_-]*\.[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\b|nyx(?:id)?_[A-Za-z0-9_-]{8,}|\b(?:AKIA|ASIA|ABIA|ACCA)[A-Z0-9]{16}\b|\bsk[-_][A-Za-z0-9_-]{8,}\b|\bgh[pousr]_[A-Za-z0-9]{20,}\b|\bAIza[A-Za-z0-9_-]{30,}\b|\bxox[baprs]-[A-Za-z0-9-]{10,}\b/gi;

const SECRET_ASSIGNMENT_PATTERN =
  /(["']?[\w.-]*(?:authorization|api[-_]?key|access[-_]?key[-_]?id|token|secret|password|credential|cookie)[\w.-]*["']?\s*[:=]\s*)("[^"]*"|'[^']*'|[^",'\s}]+)/gi;

export function redactAssistantDisplayText(value: string): string {
  return value
    .replace(SECRET_VALUE_PATTERN, (_match, bearerPrefix: unknown) =>
      typeof bearerPrefix === "string" && bearerPrefix
        ? `${bearerPrefix}[redacted]`
        : "[redacted]",
    )
    .replace(SECRET_ASSIGNMENT_PATTERN, '$1"[redacted]"');
}

export function safeAssistantDisplayText(
  value: unknown,
  fallback: string,
  maxLength = 1_024,
): string {
  if (typeof value !== "string" || !value.trim()) return fallback;
  return redactAssistantDisplayText(value.trim()).slice(0, maxLength);
}

export function humanizeAssistantServiceSlug(slug: string): string {
  const bare = slug.replace(/^api-/, "");
  if (!bare) return "NyxID service";
  return bare
    .split(/[-_]/)
    .map((part) => (part ? part.charAt(0).toUpperCase() + part.slice(1) : part))
    .join(" ");
}
