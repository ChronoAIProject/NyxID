import * as SecureStore from "expo-secure-store";
import { DOMAIN_SETS, selectNewDomains } from "./domains";

const SELECTION_KEY = "nyxid_mobile_new_domains_v1";
export const DOMAIN_FLAG_TIMEOUT_MS = 5000;

export type DomainBootstrapDeps = {
  fetch: typeof fetch;
  getItem: (key: string) => Promise<string | null>;
  setItem: (key: string, value: string) => Promise<void>;
  timeoutMs?: number;
};

const defaultDeps: DomainBootstrapDeps = {
  fetch: (input, init) => fetch(input, init),
  getItem: (key) => SecureStore.getItemAsync(key),
  setItem: (key, value) => SecureStore.setItemAsync(key, value),
};

async function readFlag(
  apiBaseUrl: string,
  signal: AbortSignal,
  fetchImpl: typeof fetch,
): Promise<boolean | undefined> {
  try {
    const response = await fetchImpl(`${apiBaseUrl}/public/config`, {
      headers: { Accept: "application/json" },
      cache: "no-store",
      signal,
    });
    if (!response.ok) return undefined;
    const body: unknown = await response.json();
    const value =
      body && typeof body === "object"
        ? (body as { mobile_new_domains?: unknown }).mobile_new_domains
        : undefined;
    // Older backends omit the field; that is no answer, not "off".
    return typeof value === "boolean" ? value : undefined;
  } catch {
    return undefined;
  }
}

/** Query every host at once and settle on the first valid answer. */
function firstValidFlag(
  apiBaseUrls: readonly string[],
  deps: DomainBootstrapDeps,
): Promise<boolean | undefined> {
  const controller = new AbortController();
  return new Promise((resolve) => {
    let settled = false;
    let remaining = apiBaseUrls.length;
    const finish = (value: boolean | undefined) => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      controller.abort();
      resolve(value);
    };
    const timer = setTimeout(
      () => finish(undefined),
      deps.timeoutMs ?? DOMAIN_FLAG_TIMEOUT_MS,
    );
    for (const apiBaseUrl of apiBaseUrls) {
      void readFlag(apiBaseUrl, controller.signal, deps.fetch).then((value) => {
        if (value !== undefined) finish(value);
        else if (--remaining === 0) finish(undefined);
      });
    }
  });
}

/**
 * Select the production domain set from the `mobile:new-domains` admin flag.
 * Runs once per launch before the app renders. Both API hosts are asked in
 * parallel and the first valid answer wins. When neither answers in time, the
 * last answer saved on this device applies, else the original domains.
 */
export async function initializeDomains(
  deps: DomainBootstrapDeps = defaultDeps,
): Promise<void> {
  const next = DOMAIN_SETS.new;
  if (!next) return;

  const [remote, saved] = await Promise.all([
    firstValidFlag([DOMAIN_SETS.legacy.apiBaseUrl, next.apiBaseUrl], deps),
    deps.getItem(SELECTION_KEY).catch(() => null),
  ]);
  if (remote === undefined) {
    selectNewDomains(saved === "true");
    return;
  }
  selectNewDomains(remote);
  if (saved !== String(remote)) {
    await deps.setItem(SELECTION_KEY, String(remote)).catch(() => {});
  }
}
