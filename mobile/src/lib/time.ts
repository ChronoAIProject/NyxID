import { appLocale, translateApp } from "../i18n/appTranslation";
export function formatRelativeTimeFromMs(
  timestampMs: number | null | undefined,
  nowMs: number = Date.now(),
  fallback: string = "Not synced"
): string {
  if (!timestampMs || !Number.isFinite(timestampMs) || timestampMs <= 0) {
    return translateApp(fallback);
  }

  const diffMs = Math.max(0, nowMs - timestampMs);
  const diffSeconds = Math.floor(diffMs / 1000);
  const diffMinutes = Math.floor(diffSeconds / 60);
  const diffHours = Math.floor(diffMinutes / 60);
  const diffDays = Math.floor(diffHours / 24);

  // Hermes ships no Intl.RelativeTimeFormat (`new` on it throws), so every app
  // language uses complete catalog phrases.
  if (diffSeconds < 60) return translateApp("Just now");
  if (diffMinutes < 60)
    return translateApp("{{minutes}}m ago", { minutes: diffMinutes });
  if (diffHours < 24) return translateApp("{{hours}}h ago", { hours: diffHours });
  if (diffDays < 7) return translateApp("{{days}}d ago", { days: diffDays });

  return new Date(timestampMs).toLocaleDateString(appLocale(), {
    month: "short",
    day: "numeric",
    year: "numeric",
  });
}
