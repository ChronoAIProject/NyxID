const KEY = "nyxid:personal-link:v1";
const MAX_AGE = 24 * 60 * 60 * 1000;
const tokenPattern = /^nyx_pli_[a-f0-9]{64}$/i;
type Handoff = {
  token: string;
  id: string;
  expiresAt: number;
  challenge?: { id: string; accountId: string; expiresAt: number };
};
let incoming: Handoff | null = null;

function valid(value: unknown): value is Handoff {
  if (!value || typeof value !== "object") return false;
  const row = value as Partial<Handoff>;
  return (
    typeof row.token === "string" &&
    tokenPattern.test(row.token) &&
    typeof row.id === "string" &&
    typeof row.expiresAt === "number" &&
    row.expiresAt > Date.now() &&
    row.expiresAt <= Date.now() + MAX_AGE
  );
}
function save(row: Handoff) {
  incoming = row;
  try {
    sessionStorage.setItem(KEY, JSON.stringify(row));
  } catch {
    /* Memory-only continuation remains possible in this tab. */
  }
}
export function clearPersonalLink() {
  incoming = null;
  try {
    sessionStorage.removeItem(KEY);
  } catch {
    /* Storage may be blocked. */
  }
}
export function readPersonalLink(): Handoff | null {
  if (valid(incoming)) return incoming;
  try {
    const row: unknown = JSON.parse(sessionStorage.getItem(KEY) ?? "null");
    if (valid(row)) {
      incoming = row;
      return row;
    }
  } catch {
    /* No usable handoff. */
  }
  clearPersonalLink();
  return null;
}
export function shortenPersonalLink(expiresAt: string) {
  const row = readPersonalLink();
  if (row)
    save({ ...row, expiresAt: Math.min(row.expiresAt, Date.parse(expiresAt)) });
}
export function savePersonalChallenge(
  id: string,
  accountId: string,
  expiresAt: string,
) {
  const row = readPersonalLink();
  if (row)
    save({
      ...row,
      challenge: { id, accountId, expiresAt: Date.parse(expiresAt) },
    });
}
export function forgetPersonalChallenge() {
  const row = readPersonalLink();
  if (row) {
    const rest = { ...row };
    delete rest.challenge;
    save(rest);
  }
}
export function capturePersonalLink() {
  const url = new URL(window.location.href);
  const queryToken = url.searchParams.has("intent_token");
  if (
    !queryToken &&
    (!url.hash ||
      (url.pathname !== "/nyxbot/link" && !url.hash.includes("intent_token")))
  )
    return;
  const fragment = new URLSearchParams(url.hash.slice(1));
  const token = fragment.get("intent_token");
  // Scrub before storage, imports with side effects, or any authentication work.
  url.searchParams.delete("intent_token");
  window.history.replaceState(
    window.history.state,
    "",
    url.pathname + url.search,
  );
  clearPersonalLink();
  if (
    !queryToken &&
    url.pathname === "/nyxbot/link" &&
    fragment.size === 1 &&
    token &&
    tokenPattern.test(token)
  )
    save({ token, id: crypto.randomUUID(), expiresAt: Date.now() + MAX_AGE });
}
if (typeof window !== "undefined") capturePersonalLink();
