/** Native follows the device's language; supplied explicitly to title generation. */
export function assistantTitleLanguage(): string {
  try { return Intl.DateTimeFormat().resolvedOptions().locale || "en"; } catch { return "en"; }
}
