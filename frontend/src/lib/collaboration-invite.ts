// Keep a bearer invite out of login return_to URLs, referrers and router state.
// Tab-local storage carries it through sign-in; the server checks real expiry.
const KEY = "nyxid:collaboration-invite:v1";
const MAX_AGE = 60 * 60 * 1000;
let pending: { token: string; expires: number } | undefined;
export function clearGroupInvite() {
  pending = undefined;
  try {
    sessionStorage.removeItem(KEY);
  } catch {
    /* Tab storage may be unavailable. */
  }
}
export function captureGroupInvite() {
  if (window.location.pathname !== "/assistant") return;
  const params = new URLSearchParams(window.location.hash.slice(1));
  if (!params.has("group-invite")) return;
  const values = params.getAll("group-invite");
  params.delete("group-invite");
  const remaining = params.toString();
  window.history.replaceState(
    window.history.state,
    "",
    window.location.pathname +
      window.location.search +
      (remaining ? `#${remaining}` : ""),
  );
  clearGroupInvite();
  const token = values.length === 1 ? values[0] : undefined;
  if (!token || !/^[a-f0-9]{64}$/.test(token)) return;
  pending = { token, expires: Date.now() + MAX_AGE };
  try {
    sessionStorage.setItem(KEY, JSON.stringify(pending));
  } catch {
    /* Continue in memory. */
  }
}
export function readGroupInvite(): string | null {
  captureGroupInvite();
  try {
    const value: unknown =
      pending ?? JSON.parse(sessionStorage.getItem(KEY) ?? "null");
    if (
      value &&
      typeof value === "object" &&
      "token" in value &&
      "expires" in value &&
      typeof value.token === "string" &&
      /^[a-f0-9]{64}$/.test(value.token) &&
      typeof value.expires === "number" &&
      value.expires > Date.now() &&
      value.expires <= Date.now() + MAX_AGE
    )
      return value.token;
  } catch {
    /* Malformed or unavailable storage. */
  }
  clearGroupInvite();
  return null;
}
