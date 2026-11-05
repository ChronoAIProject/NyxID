/**
 * Production domain sets embedded at build time by app.config.ts.
 *
 * Builds configured with all three `PROD_NEW_*` URLs carry a second set. The
 * launch gate (`app/DomainGate.tsx`) reads the NyxID admin flag
 * `mobile:new-domains` and calls `selectNewDomains` once, before the app
 * renders. Accessors read the private selection when a URL is built; they do
 * not depend on bundler support for mutable exports. The server only
 * chooses between these embedded sets and never supplies a URL.
 */
export type DomainSet = Readonly<{
  apiBaseUrl: string;
  frontendUrl: string;
  legalBaseUrl: string;
}>;

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
  return Object.freeze({
    apiBaseUrl: normalizeApiBaseUrl(apiBaseUrl),
    frontendUrl: (frontendUrl ?? "").trim(),
    legalBaseUrl: (legalBaseUrl ?? "").trim().replace(/\/+$/, ""),
  });
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

// A single-set build is ready immediately. Dual-set builds must pass the gate.
let selectedDomains: DomainSet | null = NEW_DOMAINS ? null : LEGACY_DOMAINS;

export function activeDomains(): DomainSet {
  if (!selectedDomains) {
    throw new Error("Domains have not been selected. Wait for DomainGate before building URLs.");
  }
  return selectedDomains;
}

export function getFrontendUrl(): string {
  return activeDomains().frontendUrl;
}

export function getLegalBaseUrl(): string {
  return activeDomains().legalBaseUrl;
}

export function selectNewDomains(enabled: boolean): void {
  selectedDomains = enabled && NEW_DOMAINS ? NEW_DOMAINS : LEGACY_DOMAINS;
}
