const NORMALIZED_USER_CODE_LENGTH = 8;
const NORMALIZED_USER_CODE_PATTERN = /^[0-9A-HJKMNP-TV-Z]{8}$/;

export function normalizeAuthDeviceUserCode(raw: string): string | null {
  const compact = raw.replace(/[- \t]/g, "").toUpperCase();
  const normalized = compact
    .replace(/[IL]/g, "1")
    .replace(/O/g, "0")
    .replace(/U/g, "V");

  if (
    normalized.length !== NORMALIZED_USER_CODE_LENGTH ||
    !NORMALIZED_USER_CODE_PATTERN.test(normalized)
  ) {
    return null;
  }

  return normalized;
}

export function formatAuthDeviceUserCode(raw: string): string {
  const compact = raw
    .replace(/[- \t]/g, "")
    .toUpperCase()
    .slice(0, NORMALIZED_USER_CODE_LENGTH);

  return compact.length > 4
    ? `${compact.slice(0, 4)}-${compact.slice(4)}`
    : compact;
}

export function extractAuthDeviceUserCodeFromQr(raw: string): string | null {
  let parsed: URL;
  try {
    parsed = new URL(raw.trim());
  } catch {
    return null;
  }

  const isWebLogin =
    parsed.protocol === "https:" &&
    (parsed.pathname === "/login/device" || parsed.pathname === "/login/device/");
  const isAppLogin =
    parsed.protocol === "nyxid:" &&
    ((parsed.hostname.toLowerCase() === "login" &&
      (parsed.pathname === "/device" || parsed.pathname === "/device/")) ||
      (parsed.hostname === "" &&
        (parsed.pathname === "/login/device" || parsed.pathname === "/login/device/")));

  if (!isWebLogin && !isAppLogin) return null;

  const userCodes = parsed.searchParams.getAll("user_code");
  if (userCodes.length !== 1) return null;

  return normalizeAuthDeviceUserCode(userCodes[0] ?? "");
}
