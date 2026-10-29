/**
 * Production domain sets embedded at build time by app.config.ts.
 *
 * Builds configured with all three `PROD_NEW_*` URLs carry a second set. The
 * launch gate (`app/DomainGate.tsx`) reads the NyxID admin flag
 * `mobile:new-domains` and calls `selectNewDomains` once, before the app
 * renders. The exported `NYXID_*` bindings then name the active set; they are
 * live bindings, so importers read the selected values. The server only
 * chooses between these embedded sets and never supplies a URL.
 */
export type DomainSet = {
  apiBaseUrl: string;
  frontendUrl: string;
  legalBaseUrl: string;
};

function normalizeApiBaseUrl(configuredUrl: string | undefined): string {
  const normalized = (configuredUrl ?? "").trim().replace(/\/+$/, "");
  return normalized.endsWith("/mobile")
    ? normalized.slice(0, -"/mobile".length)
    : normalized;
}

function domainSet(
  apiBaseUrl: string | undefined,
  frontendUrl: string | undefined,
  legalBaseUrl: string | undefined,
): DomainSet {
  return {
    apiBaseUrl: normalizeApiBaseUrl(apiBaseUrl),
    frontendUrl: (frontendUrl ?? "").trim(),
    legalBaseUrl: (legalBaseUrl ?? "").trim().replace(/\/+$/, ""),
  };
}

const LEGACY_DOMAINS = domainSet(
  process.env.EXPO_PUBLIC_API_BASE_URL,
  process.env.EXPO_PUBLIC_FRONTEND_URL,
  process.env.EXPO_PUBLIC_LEGAL_BASE_URL,
);

const CONFIGURED_NEW_DOMAINS = domainSet(
  process.env.EXPO_PUBLIC_NEW_API_BASE_URL,
  process.env.EXPO_PUBLIC_NEW_FRONTEND_URL,
  process.env.EXPO_PUBLIC_NEW_LEGAL_BASE_URL,
);

const NEW_DOMAINS: DomainSet | null =
  CONFIGURED_NEW_DOMAINS.apiBaseUrl &&
  CONFIGURED_NEW_DOMAINS.frontendUrl &&
  CONFIGURED_NEW_DOMAINS.legalBaseUrl
    ? CONFIGURED_NEW_DOMAINS
    : null;

export const DOMAIN_SETS: { readonly legacy: DomainSet; readonly new: DomainSet | null } = {
  legacy: LEGACY_DOMAINS,
  new: NEW_DOMAINS,
};

/** Every embedded set. QR trust and link routing accept all of them. */
export const CONFIGURED_DOMAIN_SETS: readonly DomainSet[] = NEW_DOMAINS
  ? [LEGACY_DOMAINS, NEW_DOMAINS]
  : [LEGACY_DOMAINS];

export let NYXID_API_BASE_URL = LEGACY_DOMAINS.apiBaseUrl;
export let NYXID_FRONTEND_URL = LEGACY_DOMAINS.frontendUrl;
export let NYXID_LEGAL_BASE_URL = LEGACY_DOMAINS.legalBaseUrl;
export let NYXID_USING_NEW_DOMAINS = false;

export function selectNewDomains(enabled: boolean): void {
  const selected = enabled && NEW_DOMAINS ? NEW_DOMAINS : LEGACY_DOMAINS;
  NYXID_USING_NEW_DOMAINS = selected === NEW_DOMAINS;
  NYXID_API_BASE_URL = selected.apiBaseUrl;
  NYXID_FRONTEND_URL = selected.frontendUrl;
  NYXID_LEGAL_BASE_URL = selected.legalBaseUrl;
}
