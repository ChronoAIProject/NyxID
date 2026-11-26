import { Platform, Settings } from "react-native";

/** iOS preferred languages precede the bundle-dependent current NSLocale. */
export function deviceLanguages({
  includeLocaleFallback = false,
}: { includeLocaleFallback?: boolean } = {}): string[] {
  const preferred: unknown =
    Platform.OS === "ios" ? Settings.get("AppleLanguages") : undefined;
  const languages = Array.isArray(preferred)
    ? preferred.filter(
        (value): value is string => typeof value === "string" && !!value.trim(),
      )
    : [];
  // Once iOS supplies its preferred list, Intl's en-CN/en-JP bundle locale
  // must never introduce an unrelated English recognizer.
  if (languages.length && !includeLocaleFallback) return languages;
  try {
    return [...languages, Intl.DateTimeFormat().resolvedOptions().locale];
  } catch {
    return languages;
  }
}
