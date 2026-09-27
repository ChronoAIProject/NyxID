import { parseLoginRequestHints } from "@/schemas/login-request";
const PREFIX = "nyxid:device-identity:";
const MAX_AGE_MS = 10 * 60 * 1000;
const tokenPattern = /^[0-9a-f-]{36}$/;

/** Called only by the explicit identity-verification click. */
export function saveLoginResume(
  token: string,
  flow: "device" | "agent-key",
  query: string,
) {
  if (
    !tokenPattern.test(token) ||
    parseLoginRequestHints(query, flow).errors.length
  )
    throw new Error(
      "This request link cannot be resumed. Check its parameters.",
    );
  sessionStorage.setItem(
    `${PREFIX}${token}`,
    JSON.stringify({ flow, query, expires: Date.now() + MAX_AGE_MS }),
  );
}

export function resolveLoginResume(
  flow: "device" | "agent-key",
  query: string,
): { query: string; error?: string; token?: string } {
  const params = new URLSearchParams(query);
  if (!params.has("resume")) return { query };
  const token = params.get("resume") ?? "";
  try {
    if ([...params.keys()].length !== 1 || !tokenPattern.test(token))
      throw new Error();
    const stored: unknown = JSON.parse(
      sessionStorage.getItem(`${PREFIX}${token}`) ?? "null",
    );
    if (
      !stored ||
      typeof stored !== "object" ||
      !("flow" in stored) ||
      stored.flow !== flow ||
      !("query" in stored) ||
      typeof stored.query !== "string" ||
      !("expires" in stored) ||
      typeof stored.expires !== "number" ||
      stored.expires <= Date.now() ||
      stored.expires > Date.now() + MAX_AGE_MS ||
      parseLoginRequestHints(stored.query, flow).errors.length
    )
      throw new Error();
    return { query: stored.query, token };
  } catch {
    return {
      query: "",
      error:
        "The saved request is unavailable or expired. Reopen the original request link in this browser.",
    };
  }
}
export function clearLoginResume(token: string | undefined) {
  try {
    if (token) sessionStorage.removeItem(`${PREFIX}${token}`);
  } catch {
    /* Storage access can be withdrawn after sign-in. */
  }
}
