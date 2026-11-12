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

type HostKey = "legacy" | "new";
type HostAnswer = {
  /** The host served a successful response, so requests can reach it. */
  reachable: boolean;
  /** Older backends omit the field; that is no answer, not "off". */
  flag: boolean | undefined;
};

async function probeHost(
  apiBaseUrl: string,
  signal: AbortSignal,
  fetchImpl: typeof fetch,
): Promise<HostAnswer> {
  try {
    const response = await fetchImpl(`${apiBaseUrl}/public/config`, {
      headers: { Accept: "application/json" },
      cache: "no-store",
      signal,
    });
    if (!response.ok) return { reachable: false, flag: undefined };
    const body: unknown = await response.json();
    const value =
      body && typeof body === "object"
        ? (body as { mobile_new_domains?: unknown }).mobile_new_domains
        : undefined;
    return { reachable: true, flag: typeof value === "boolean" ? value : undefined };
  } catch {
    return { reachable: false, flag: undefined };
  }
}

type Survey = {
  flag: boolean | undefined;
  answers: Partial<Record<HostKey, HostAnswer>>;
};

/**
 * Ask both hosts at once. Settle as soon as the flag is known and the host it
 * selects has answered (or failed), so a hung host only delays launch when it
 * is the one the flag chose; never wait past the timeout.
 */
function surveyHosts(
  hosts: Readonly<Record<HostKey, string>>,
  deps: DomainBootstrapDeps,
): Promise<Survey> {
  const controller = new AbortController();
  const answers: Partial<Record<HostKey, HostAnswer>> = {};
  let flag: boolean | undefined;
  return new Promise((resolve) => {
    let settled = false;
    const finish = () => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      controller.abort();
      resolve({ flag, answers: { ...answers } });
    };
    const timer = setTimeout(finish, deps.timeoutMs ?? DOMAIN_FLAG_TIMEOUT_MS);
    const keys = Object.keys(hosts) as HostKey[];
    for (const key of keys) {
      void probeHost(hosts[key], controller.signal, deps.fetch).then((answer) => {
        if (settled) return;
        answers[key] = answer;
        if (flag === undefined && answer.flag !== undefined) flag = answer.flag;
        const chosen: HostKey | undefined =
          flag === undefined ? undefined : flag ? "new" : "legacy";
        if (keys.every((k) => answers[k])) finish();
        else if (chosen && answers[chosen]) finish();
      });
    }
  });
}

/**
 * Select the production domain set from the `mobile:new-domains` admin flag,
 * then confirm that set's API host is reachable. Runs once per launch before
 * the app renders, so no request is built before a working base URL is known.
 * Both hosts are asked in parallel. When neither answers in time, the last
 * answer saved on this device applies, else the original domains. When the
 * chosen host is unreachable but the other answered, this launch uses the
 * reachable set (both serve the same backend and accept the same sessions);
 * the saved choice still records only the flag. Storage failures never replace
 * a remote answer; every exit selects a domain set.
 */
export async function initializeDomains(
  deps: DomainBootstrapDeps = defaultDeps,
): Promise<void> {
  let survey: Survey = { flag: undefined, answers: {} };
  let saved: string | null = null;
  let enabled = false;
  try {
    const next = DOMAIN_SETS.new;
    if (!next) return;

    // Isolate synchronous throws and rejections without losing another result.
    const savedAnswer = Promise.resolve()
      .then(() => deps.getItem(SELECTION_KEY))
      .then((value) => { saved = value; }, () => {});
    const remoteAnswer = Promise.resolve()
      .then(() =>
        surveyHosts({ legacy: DOMAIN_SETS.legacy.apiBaseUrl, new: next.apiBaseUrl }, deps),
      )
      .then((value) => { survey = value; }, () => {});
    await Promise.all([remoteAnswer, savedAnswer]);
    const remote = survey.flag;
    if (remote !== undefined && saved !== String(remote)) {
      await Promise.resolve()
        .then(() => deps.setItem(SELECTION_KEY, String(remote)))
        .catch(() => {});
    }
    enabled = remote ?? (saved === "true");
    const chosen: HostKey = enabled ? "new" : "legacy";
    const other: HostKey = enabled ? "legacy" : "new";
    if (!survey.answers[chosen]?.reachable && survey.answers[other]?.reachable)
      enabled = other === "new";
  } finally {
    selectNewDomains(enabled);
  }
}
