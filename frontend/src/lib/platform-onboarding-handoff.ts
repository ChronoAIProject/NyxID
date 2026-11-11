const KEY = "nyxid:onboarding-link:v2";
const MAX_AGE = 10 * 60 * 1000;
type Handoff = { code: string; id: string; expiresAt: number };
let incoming: Handoff | undefined;
const listeners = new Set<(handoff: Handoff | undefined) => void>();
export function subscribeOnboardingLink(
  listener: (handoff: Handoff | undefined) => void,
) {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}
function notifyIncomingLink() {
  for (const listener of listeners) listener(incoming);
}

function valid(value: unknown): value is Handoff {
  if (!value || typeof value !== "object") return false;
  const row = value as Partial<Handoff>;
  return (
    typeof row.code === "string" &&
    /^[A-Z0-9]{8}$/.test(row.code) &&
    typeof row.id === "string" &&
    typeof row.expiresAt === "number" &&
    row.expiresAt > Date.now() &&
    row.expiresAt <= Date.now() + MAX_AGE
  );
}
export function clearOnboardingLink() {
  incoming = undefined;
  try {
    sessionStorage.removeItem(KEY);
  } catch {
    /* This tab may have memory-only storage. */
  }
}
export function readOnboardingLink(): Handoff | undefined {
  if (valid(incoming)) return incoming;
  try {
    const saved: unknown = JSON.parse(sessionStorage.getItem(KEY) ?? "null");
    if (valid(saved)) return (incoming = saved);
  } catch {
    /* Invalid or unavailable session storage. */
  }
  clearOnboardingLink();
  return undefined;
}
export function captureOnboardingLink() {
  const url = new URL(window.location.href);
  if (url.pathname !== "/onboarding/link") return;
  const fragment = new URLSearchParams(url.hash.slice(1));
  if (!url.searchParams.has("code") && !fragment.has("code")) return;
  const codes = [
    ...url.searchParams.getAll("code"),
    ...fragment.getAll("code"),
  ];
  url.searchParams.delete("code");
  window.history.replaceState(
    window.history.state,
    "",
    url.pathname + url.search,
  );
  clearOnboardingLink();
  if (codes.length !== 1) {
    notifyIncomingLink();
    return;
  }
  const code = codes[0]?.replace(/[-\s]/g, "").toUpperCase();
  if (!code || !/^[A-Z0-9]{8}$/.test(code)) {
    notifyIncomingLink();
    return;
  }
  incoming = { code, id: crypto.randomUUID(), expiresAt: Date.now() + MAX_AGE };
  try {
    sessionStorage.setItem(KEY, JSON.stringify(incoming));
  } catch {
    /* Continue in memory. */
  }
  notifyIncomingLink();
}
export function onboardingConfirmationUrl(
  command: string | null | undefined,
  username: string | null | undefined,
) {
  if (!command || !username || !/^[A-Za-z0-9_]{5,32}$/.test(username))
    return undefined;
  const code = /^\/confirm (O-[A-F0-9]{32})$/.exec(command)?.[1];
  return code ? `https://t.me/${username}?start=nyxonboard_${code}` : undefined;
}
if (typeof window !== "undefined") captureOnboardingLink();
