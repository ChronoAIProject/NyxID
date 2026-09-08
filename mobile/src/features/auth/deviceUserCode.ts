import {
  IS_DEV_BUILD,
  NYXID_API_BASE_URL,
  NYXID_APP_SCHEME,
  NYXID_FRONTEND_URL,
  NYXID_UNIVERSAL_LINK_HOST,
} from "../../lib/env";

const NORMALIZED_USER_CODE_LENGTH = 8;
const NORMALIZED_USER_CODE_PATTERN = /^(?:2)?[0-9A-HJKMNP-TV-Z]{8}$/;
const ABSOLUTE_URL_PATTERN =
  /^([a-z][a-z0-9+.-]*):\/\/([^/?#]*)(\/[^?#]*)?(?:\?([^#]*))?(?:#.*)?$/i;

export type AuthDeviceQrTrustPolicy = {
  appScheme: string;
  webOrigins: readonly string[];
  allowHttp: boolean;
};

function defaultWebOrigins(): string[] {
  const origins = [NYXID_FRONTEND_URL, NYXID_API_BASE_URL];
  if (NYXID_UNIVERSAL_LINK_HOST) {
    origins.push(`https://${NYXID_UNIVERSAL_LINK_HOST}`);
    if (IS_DEV_BUILD) origins.push(`http://${NYXID_UNIVERSAL_LINK_HOST}`);
  }
  return origins.filter(Boolean);
}

const DEFAULT_QR_TRUST_POLICY: AuthDeviceQrTrustPolicy = {
  appScheme: NYXID_APP_SCHEME,
  webOrigins: defaultWebOrigins(),
  allowHttp: IS_DEV_BUILD,
};

function canonicalAuthority(authority: string, protocol: "http" | "https"): string | null {
  if (!authority || /[\s\\@%]/.test(authority)) return null;

  let host: string;
  let port = "";
  if (authority.startsWith("[")) {
    const ipv6 = authority.match(/^\[([0-9a-f:.]+)\](?::([0-9]+))?$/i);
    if (!ipv6?.[1]) return null;
    host = `[${ipv6[1].toLowerCase()}]`;
    port = ipv6[2] ?? "";
  } else {
    const parts = authority.split(":");
    if (parts.length > 2) return null;
    host = (parts[0] ?? "").toLowerCase();
    port = parts[1] ?? "";
    if (
      !host ||
      !/^(?:localhost|(?:[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?)(?:\.(?:[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?))*)$/.test(host)
    ) {
      return null;
    }
  }

  if (port && (!/^\d{1,5}$/.test(port) || Number(port) > 65535)) return null;
  if ((protocol === "https" && port === "443") || (protocol === "http" && port === "80")) {
    port = "";
  }
  return port ? `${host}:${port}` : host;
}

function canonicalConfiguredOrigin(value: string): string | null {
  const match = value.trim().match(ABSOLUTE_URL_PATTERN);
  if (!match?.[1] || !match[2]) return null;
  const protocol = match[1].toLowerCase();
  if (protocol !== "http" && protocol !== "https") return null;
  const authority = canonicalAuthority(match[2], protocol);
  return authority ? `${protocol}://${authority}` : null;
}

function decodeQueryComponent(value: string): string | null {
  try {
    return decodeURIComponent(value.replace(/\+/g, " "));
  } catch {
    return null;
  }
}

function extractSingleUserCode(query: string | undefined): string | null {
  if (query === undefined) return null;

  const values: string[] = [];
  for (const pair of query.split("&")) {
    const separator = pair.indexOf("=");
    const rawKey = separator >= 0 ? pair.slice(0, separator) : pair;
    const rawValue = separator >= 0 ? pair.slice(separator + 1) : "";
    const key = decodeQueryComponent(rawKey);
    const value = decodeQueryComponent(rawValue);
    if (key === null || value === null) return null;
    if (key === "user_code") values.push(value);
  }

  return values.length === 1 ? normalizeAuthDeviceUserCode(values[0] ?? "") : null;
}

export function normalizeAuthDeviceUserCode(raw: string): string | null {
  const compact = raw.replace(/[- \t]/g, "").toUpperCase();
  const normalized = compact
    .replace(/[IL]/g, "1")
    .replace(/O/g, "0")
    .replace(/U/g, "V");

  if (
    !NORMALIZED_USER_CODE_PATTERN.test(normalized)
  ) {
    return null;
  }

  return normalized;
}

export function supportsRestrictedDeviceLogin(raw: string): boolean {
  return normalizeAuthDeviceUserCode(raw)?.length === 9;
}

export function formatAuthDeviceUserCode(raw: string): string {
  const compact = raw
    .replace(/[- \t]/g, "")
    .toUpperCase()
    .slice(0, raw.replace(/[- \t]/g, "").startsWith("2") ? 9 : NORMALIZED_USER_CODE_LENGTH);

  if (compact.length === 9 && compact.startsWith("2")) {
    return `2-${compact.slice(1, 5)}-${compact.slice(5)}`;
  }

  return compact.length > 4
    ? `${compact.slice(0, 4)}-${compact.slice(4)}`
    : compact;
}

export function extractAuthDeviceUserCodeFromQr(
  raw: string,
  trustPolicy: AuthDeviceQrTrustPolicy = DEFAULT_QR_TRUST_POLICY
): string | null {
  const result = extractLoginRequestFromQr(raw, trustPolicy);
  return result?.kind === "device" ? result.userCode : null;
}

export function extractLoginRequestFromQr(
  raw: string,
  trustPolicy: AuthDeviceQrTrustPolicy = DEFAULT_QR_TRUST_POLICY,
): { kind: "device" | "agent-key"; userCode: string } | null {
  const candidate = raw.trim();
  if (!candidate || /[\u0000-\u001f\u007f\\]/.test(candidate)) return null;

  const match = candidate.match(ABSOLUTE_URL_PATTERN);
  if (!match?.[1]) return null;

  const scheme = match[1].toLowerCase();
  const authority = match[2] ?? "";
  const path = match[3] ?? "/";
  const query = match[4];
  const loginPath = scheme === trustPolicy.appScheme.toLowerCase() && authority.toLowerCase() === "login"
    ? `/login${path}` : path;
  const kind = /^\/login\/agent-key\/?$/.test(loginPath) ? "agent-key" : /^\/login\/device\/?$/.test(loginPath) ? "device" : null;
  const userCode = extractSingleUserCode(query);
  if (!kind || !userCode) return null;

  if (scheme === trustPolicy.appScheme.toLowerCase()) {
    return authority.toLowerCase() === "login" || authority === "" ? { kind, userCode } : null;
  }

  if (scheme !== "https" && scheme !== "http") return null;
  if (scheme === "http" && !trustPolicy.allowHttp) return null;

  const canonicalHost = canonicalAuthority(authority, scheme);
  if (!canonicalHost) return null;
  const candidateOrigin = `${scheme}://${canonicalHost}`;
  const trusted = trustPolicy.webOrigins.some(
    (origin) => canonicalConfiguredOrigin(origin) === candidateOrigin
  );
  return trusted ? { kind, userCode } : null;
}
