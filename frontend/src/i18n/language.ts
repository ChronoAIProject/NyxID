export const LANGUAGES = [
  { code: "en", label: "English", shortLabel: "EN" },
  { code: "zh-CN", label: "简体中文", shortLabel: "简体" },
  { code: "zh-TW", label: "繁體中文", shortLabel: "繁體" },
] as const;

export type Language = (typeof LANGUAGES)[number]["code"];
export const LANGUAGE_STORAGE_KEY = "nyxid.language";

export function resolveLanguage(
  value: string | null | undefined,
): Language | undefined {
  if (!value) return undefined;
  try {
    const locale = new Intl.Locale(value.replaceAll("_", "-"));
    if (locale.language === "en") return "en";
    if (locale.language === "zh") {
      return locale.maximize().script === "Hant" ? "zh-TW" : "zh-CN";
    }
  } catch {
    // Invalid or unsupported stored values fall back to browser preferences.
  }
  return undefined;
}

export function initialLanguage(): Language {
  try {
    const saved = resolveLanguage(localStorage.getItem(LANGUAGE_STORAGE_KEY));
    if (saved) return saved;
    const legacy = resolveLanguage(
      localStorage.getItem("nyxbot-onboarding-language"),
    );
    if (legacy) return legacy;
  } catch {
    // Language switching also works when browser storage is unavailable.
  }
  if (typeof navigator !== "undefined") {
    for (const language of [
      ...(navigator.languages ?? []),
      navigator.language,
    ]) {
      const supported = resolveLanguage(language);
      if (supported) return supported;
    }
  }
  return "en";
}
