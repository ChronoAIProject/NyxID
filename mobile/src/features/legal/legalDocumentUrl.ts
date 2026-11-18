import { getLegalBaseUrl } from "../../lib/env";

export function resolveLegalDocumentUrl(docKey: "privacy" | "terms"): string | null {
  const base = getLegalBaseUrl();
  return base ? `${base}/legal/${docKey}.md` : null;
}
