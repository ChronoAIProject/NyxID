type Names = readonly [
  readonly [string, string],
  readonly [string, string],
  readonly [string, string],
  string,
];
import names from "./dictationLanguageNames.json";
import type { Language } from "../../../i18n/language";

function entry(locale: string): Names | undefined {
  const normalized = locale
    .replace(/_/g, "-")
    .replace(/^cmn(?=-|$)/i, "zh")
    .replace(/^(zh)-(Hans|Hant)-/i, "$1-")
    .toLowerCase();
  const values = Object.entries(names);
  return (values.find(([key]) => key.toLowerCase() === normalized) ??
    values.find(
      ([key]) => key.split("-")[0]!.toLowerCase() === normalized.split("-")[0],
    ))?.[1] as Names | undefined;
}
/** Bundled names work in Hermes, which has no Intl.DisplayNames. */
export function languageLabel(locale: string, ui: Language = "en"): string {
  const row = entry(locale);
  if (!row) return locale;
  const [language, region] = row[ui === "zh-CN" ? 1 : ui === "zh-TW" ? 2 : 0];
  const exact = Object.keys(names).some(
    (key) =>
      key.toLowerCase() ===
      locale
        .replace(/_/g, "-")
        .replace(/^cmn(?=-|$)/i, "zh")
        .replace(/^(zh)-(Hans|Hant)-/i, "$1-")
        .toLowerCase(),
  );
  if (!exact || !region) return language;
  return ui === "en" ? `${language} (${region})` : `${language}（${region}）`;
}
export function languageOptions(
  supported: readonly string[],
  ui: Language = "en",
) {
  return [...new Set(supported)]
    .map((value) => ({
      value,
      label: languageLabel(value, ui),
      detail: `\u2068${entry(value)?.[3] ?? value.split(/[-_]/)[0]}\u2069 · ${value}`,
      english: languageLabel(value, "en"),
    }))
    .sort((a, b) => a.label.localeCompare(b.label, ui));
}
export function matchesLanguage(
  option: { value: string; label: string; detail?: string; english?: string },
  search: string,
) {
  const normalize = (value: string) =>
    value
      .replace(/[\u2068\u2069]/g, "")
      .normalize("NFKC")
      .toLowerCase();
  return normalize(
    [option.label, option.detail, option.english, option.value].join(" "),
  ).includes(normalize(search.trim()));
}
