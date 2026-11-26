import { translateApp } from "../../i18n/appTranslation";
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
  if (!Number.isFinite(timestamp)) return translateApp("Unknown");
  const seconds = Math.max(0, Math.floor((nowMs - timestamp) / 1000));
  // Hermes has no Intl.RelativeTimeFormat; catalog phrases cover every language.
  if (seconds < 60)
    return (seconds === 1 ? translateApp("{{seconds}} second ago", { seconds: seconds }) : translateApp("{{seconds}} seconds ago", { seconds: seconds }));
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return (minutes === 1 ? translateApp("{{minutes}} minute ago", { minutes: minutes }) : translateApp("{{minutes}} minutes ago", { minutes: minutes }));
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return (hours === 1 ? translateApp("{{hours}} hour ago", { hours: hours }) : translateApp("{{hours}} hours ago", { hours: hours }));
  const days = Math.floor(hours / 24);
  return (days === 1 ? translateApp("{{days}} day ago", { days: days }) : translateApp("{{days}} days ago", { days: days }));
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
    return host ?? translateApp("Another site");
  }
  if (status === "non_http") {
    return translateApp("Non-HTTP origin");
  }
  return translateApp("Malformed origin");
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
