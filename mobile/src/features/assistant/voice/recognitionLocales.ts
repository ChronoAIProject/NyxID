/** Query the same Android service used for recognition, not the unrelated
 * on-device recognizer selected by expo-speech-recognition's empty options. */
type LocaleResult = { locales: string[]; installedLocales: string[] };
type LocaleRecognizer = {
  getDefaultRecognitionService?(): { packageName: string };
  getSupportedLocales(options: {
    androidRecognitionServicePackage?: string;
  }): Promise<LocaleResult>;
};
export const LOCALE_DISCOVERY_TIMEOUT_MS = 2500;
export async function recognitionLocales(
  recognizer: LocaleRecognizer,
  platform: string,
) {
  let service: string | undefined;
  if (platform === "android") {
    try {
      service =
        recognizer.getDefaultRecognitionService?.().packageName || undefined;
    } catch {
      /* An unavailable service still gets a bounded discovery attempt. */
    }
  }
  let timer: ReturnType<typeof setTimeout> | undefined;
  const empty = { locales: [], installedLocales: [] };
  try {
    const result = await Promise.race([
      Promise.resolve()
        .then(() =>
          recognizer.getSupportedLocales(
            service ? { androidRecognitionServicePackage: service } : {},
          ),
        )
        .catch(() => empty),
      new Promise<LocaleResult>((resolve) => {
        timer = setTimeout(() => resolve(empty), LOCALE_DISCOVERY_TIMEOUT_MS);
      }),
    ]);
    return {
      locales: [...new Set(result?.locales ?? [])],
      installedLocales: result?.installedLocales ?? [],
      service,
    };
  } finally {
    clearTimeout(timer);
  }
}

/** A manual choice is a language tag, not a claim the provider supports it. */
export function manualDictationLocale(value: string): string | undefined {
  const tag = value.trim().replace(/_/g, "-");
  if (!/^[a-zA-Z]{2,3}(?:-[a-zA-Z]{4})?(?:-(?:[a-zA-Z]{2}|\d{3}))?$/.test(tag))
    return undefined;
  return tag
    .split("-")
    .map((part, index) =>
      index === 0
        ? part.toLowerCase()
        : part.length === 4
          ? part[0]!.toUpperCase() + part.slice(1).toLowerCase()
          : part.toUpperCase(),
    )
    .join("-");
}
