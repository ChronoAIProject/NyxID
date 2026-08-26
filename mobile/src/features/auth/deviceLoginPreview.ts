export function resolveDeviceLoginDeadlineMs(
  expiresAt: string,
  secondsRemaining: number | null,
  nowMs = Date.now(),
): number {
  if (secondsRemaining !== null) {
    return nowMs + Math.max(0, secondsRemaining) * 1000;
  }
  const parsed = Date.parse(expiresAt);
  return Number.isFinite(parsed) ? parsed : nowMs;
}

export function secondsUntilDeviceLoginDeadline(
  deadlineMs: number,
  nowMs = Date.now(),
): number {
  return Math.max(0, Math.ceil((deadlineMs - nowMs) / 1000));
}

export function formatDeviceLoginRelativeTime(
  value: string,
  nowMs = Date.now(),
): string {
  const timestamp = Date.parse(value);
  if (!Number.isFinite(timestamp)) return "Unknown";
  const seconds = Math.max(0, Math.floor((nowMs - timestamp) / 1000));
  if (seconds < 60) return `${seconds} second${seconds === 1 ? "" : "s"} ago`;
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes} minute${minutes === 1 ? "" : "s"} ago`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours} hour${hours === 1 ? "" : "s"} ago`;
  const days = Math.floor(hours / 24);
  return `${days} day${days === 1 ? "" : "s"} ago`;
}

export function compareDeviceLoginTimezones(
  requesterTimezone: string | null | undefined,
  approvingDeviceTimezone: string | null | undefined,
): "same" | "different" | "unavailable" {
  if (!requesterTimezone || !approvingDeviceTimezone) return "unavailable";
  return requesterTimezone.toLowerCase() ===
    approvingDeviceTimezone.toLowerCase()
    ? "same"
    : "different";
}

export type DeviceLoginValueTone = "default" | "warning" | "danger";

export function resolveDeviceLoginValueTones(
  originAnomalous: boolean,
  timezoneAnomalous: boolean,
  secondsRemaining: number,
): {
  origin: DeviceLoginValueTone;
  timezone: DeviceLoginValueTone;
  expiry: DeviceLoginValueTone;
} {
  // The fixed caution sentence is already accented. Prioritize one additional
  // state-driven value so the decision screen never becomes alarm-heavy.
  const timezone =
    !originAnomalous && timezoneAnomalous ? "warning" : "default";
  const expiry =
    originAnomalous || timezoneAnomalous || secondsRemaining > 60
      ? "default"
      : secondsRemaining === 0
        ? "danger"
        : "warning";
  return {
    origin: originAnomalous ? "danger" : "default",
    timezone,
    expiry,
  };
}

export function formatDeviceLoginOriginValue(
  status: "absent" | "matched" | "mismatched" | "malformed" | "non_http",
  origin: string | null | undefined,
): string | null {
  if (status === "absent" || status === "matched") return null;
  const host = deviceLoginOriginHost(origin);
  if (status === "mismatched") {
    return host ?? "Another site";
  }
  if (status === "non_http") {
    return "Non-HTTP origin";
  }
  return "Malformed origin";
}

function deviceLoginOriginHost(
  origin: string | null | undefined,
): string | null {
  if (!origin) return null;
  try {
    return new URL(origin).host || null;
  } catch {
    return null;
  }
}
