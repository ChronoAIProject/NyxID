import { dictationLocale, supportedDictationLocale } from "./dictationLocale";

/** A supported saved choice wins; otherwise preserve device preference order. */
export function resolveDictationLanguage(
  saved: string | undefined,
  preferred: readonly string[],
  supported: readonly string[],
  platform = "ios",
): string | undefined {
  if (!supported.length)
    return saved ?? (platform === "android" ? "" : preferred[0]);
  if (saved) {
    const match = supportedDictationLocale(saved, supported);
    if (match) return match;
    if (platform === "android") return undefined;
  }
  for (const language of preferred) {
    const resolved = dictationLocale(language, supported);
    if (resolved) return resolved;
  }
  return platform === "android" ? "" : undefined;
}
