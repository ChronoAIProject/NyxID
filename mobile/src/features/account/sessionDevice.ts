/** A short, human label for a session's user agent, e.g. "Safari on iPhone". */
export function sessionDeviceLabel(userAgent?: string | null): string {
  const ua = userAgent?.trim() ?? "";
  if (!ua) return "Unknown device";
  if (/nyxid[-_ ]?cli/i.test(ua)) return "NyxID CLI";
  const device = /iPad/.test(ua)
    ? "iPad"
    : /iPhone|iOS/.test(ua)
      ? "iPhone"
      : /Android/.test(ua)
        ? "Android"
        : /Macintosh|Mac OS X/.test(ua)
          ? "Mac"
          : /Windows/.test(ua)
            ? "Windows"
            : /Linux/.test(ua)
              ? "Linux"
              : undefined;
  if (/nyxid/i.test(ua) || /Expo|okhttp|CFNetwork/i.test(ua))
    return device ? `NyxID app on ${device}` : "NyxID app";
  const browser = /Edg\//.test(ua)
    ? "Edge"
    : /OPR\/|Opera/.test(ua)
      ? "Opera"
      : /Firefox\/|FxiOS/.test(ua)
        ? "Firefox"
        : /Chrome\/|CriOS/.test(ua)
          ? "Chrome"
          : /Safari\//.test(ua)
            ? "Safari"
            : undefined;
  if (browser && device) return `${browser} on ${device}`;
  return browser ?? device ?? ua.slice(0, 40);
}
