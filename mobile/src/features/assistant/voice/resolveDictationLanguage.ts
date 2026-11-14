import { dictationLocale } from "./dictationLocale";

/** A supported saved choice wins; otherwise preserve device preference order. */
export function resolveDictationLanguage(
  saved: string | undefined,
  preferred: readonly string[],
  supported: readonly string[],
): string | undefined {
  if (saved && supported.includes(saved)) return saved;
  for (const language of preferred) {
    const resolved = dictationLocale(language, supported);
    if (resolved) return resolved;
  }
  return undefined;
}
