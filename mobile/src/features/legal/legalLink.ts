export type LegalDocumentKey = "privacy" | "terms";

export type LegalLink =
  | { kind: "document"; docKey: LegalDocumentKey }
  | { kind: "external"; url: string }
  | { kind: "unsupported" };

/** Resolve links relative to the fetched document, never the app's scheme. */
export function resolveLegalLink(
  href: string,
  documentUrl: string | null,
): LegalLink {
  try {
    const base = documentUrl ? new URL(documentUrl) : null;
    const url = base ? new URL(href, base) : new URL(href);
    if (url.username || url.password) return { kind: "unsupported" };

    if (base && url.origin === base.origin) {
      if (url.pathname === "/terms")
        return { kind: "document", docKey: "terms" };
      if (url.pathname === "/privacy")
        return { kind: "document", docKey: "privacy" };
    }
    if (url.protocol === "https:" || url.protocol === "mailto:")
      return { kind: "external", url: url.href };
  } catch {
    // An invalid or unsupported URL must never reach native Linking.
  }
  return { kind: "unsupported" };
}
