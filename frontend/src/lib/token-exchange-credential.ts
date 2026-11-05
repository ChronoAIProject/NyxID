/** Shared by the Studio add-key dialog and the hosted connect page. */

// Parse a stored `token_exchange` credential (JSON object) back into a
// plain key-value map. Returns an empty object on any parse error so the
// form stays usable even if the underlying JSON is mangled.
export function parseTokenExchangeCredential(
  credential: string,
): Readonly<Record<string, string>> {
  if (!credential) return {};
  try {
    const parsed = JSON.parse(credential);
    if (parsed && typeof parsed === "object" && !Array.isArray(parsed)) {
      const result: Record<string, string> = {};
      for (const [k, v] of Object.entries(parsed)) {
        if (typeof v === "string") result[k] = v;
      }
      return result;
    }
  } catch {
    // Fall through -- treat malformed JSON as empty.
  }
  return {};
}

// Compose a map of field values back into the JSON blob the backend
// expects in `credential_encrypted`. Returns an empty string when all
// fields are blank so the surrounding "required credential" check still
// fires.
export function composeTokenExchangeCredential(
  values: Readonly<Record<string, string>>,
): string {
  const trimmed: Record<string, string> = {};
  let anyPresent = false;
  for (const [k, v] of Object.entries(values)) {
    const t = v.trim();
    if (t) anyPresent = true;
    trimmed[k] = t;
  }
  if (!anyPresent) return "";
  return JSON.stringify(trimmed);
}
