const STORAGE_KEY = "nyxid.telegram-claim-login";
const MAX_AGE = 15 * 60_000;

type Handoff = { code: string; expiresAt: number };
let incoming: Handoff | null = null;
let expiryTimer: ReturnType<typeof setTimeout> | undefined;

function expireHandoffAt(expiresAt: number) {
  clearTimeout(expiryTimer);
  expiryTimer = setTimeout(
    clearTelegramClaimHandoff,
    Math.max(0, expiresAt - Date.now()),
  );
}

function validHandoff(value: unknown): value is Handoff {
  if (!value || typeof value !== "object") return false;
  const record = value as Partial<Handoff>;
  return (
    typeof record.code === "string" &&
    record.code.length <= 64 &&
    typeof record.expiresAt === "number" &&
    record.expiresAt > Date.now() &&
    record.expiresAt <= Date.now() + MAX_AGE
  );
}

/** Runs before router construction and auth/telemetry boot. */
export function captureTelegramClaim() {
  const url = new URL(window.location.href);
  if (!url.searchParams.has("claim")) return;
  const code = url.searchParams.get("claim") ?? "";
  url.searchParams.delete("claim");
  clearTelegramClaimHandoff();
  if (
    url.pathname === "/channel-bots" &&
    url.searchParams.get("connect") === "telegram-new"
  ) {
    url.searchParams.set("claim_entry", "true");
    if (code.length > 0 && code.length <= 64) {
      incoming = { code, expiresAt: Date.now() + MAX_AGE };
      expireHandoffAt(incoming.expiresAt);
    }
  }
  window.history.replaceState(
    window.history.state,
    "",
    url.pathname + url.search + url.hash,
  );
}

/** Only called when a signed-out visitor must leave the setup page to sign in. */
export function preserveTelegramClaimForLogin() {
  if (!incoming || !validHandoff(incoming)) return;
  try {
    sessionStorage.setItem(STORAGE_KEY, JSON.stringify(incoming));
  } catch {
    // The private manager message also contains a code for manual entry.
  }
  incoming = null;
}

/** Read without consuming during render; StrictMode may run initializers twice. */
export function readTelegramClaimHandoff(): Handoff | null {
  if (incoming && validHandoff(incoming)) return incoming;
  try {
    const stored: unknown = JSON.parse(
      sessionStorage.getItem(STORAGE_KEY) ?? "null",
    );
    return validHandoff(stored) ? stored : null;
  } catch {
    return null;
  }
}

export function clearTelegramClaimHandoff() {
  clearTimeout(expiryTimer);
  expiryTimer = undefined;
  incoming = null;
  try {
    sessionStorage.removeItem(STORAGE_KEY);
  } catch {
    /* Storage may be disabled. */
  }
}

captureTelegramClaim();
{
  const expiresAt = readTelegramClaimHandoff()?.expiresAt;
  if (expiresAt) expireHandoffAt(expiresAt);
  else clearTelegramClaimHandoff();
}
