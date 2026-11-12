export const LANGUAGES = [
  { code: "en", label: "English" },
  { code: "zh-CN", label: "简体中文" },
  { code: "zh-TW", label: "繁體中文" },
] as const;
export type Language = (typeof LANGUAGES)[number]["code"];
export const LANGUAGE_STORAGE_KEY = "nyxid.language";

/** Match the web preference's script-aware Chinese locale negotiation. */
export function resolveLanguage(
  value: string | null | undefined,
): Language | undefined {
  if (!value) return undefined;
  // Hermes does not implement Intl.Locale. Parse the supported BCP 47 tags
  // directly, keeping explicit script ahead of regional defaults as on web.
  const tag = value.replaceAll("_", "-").toLowerCase();
  const match =
    /^(en|zh)(?:-([a-z]{4}))?(?:-([a-z]{2}|[0-9]{3}))?(?:-(?:[a-z0-9]{5,8}|[0-9][a-z0-9]{3}))*(?:-[0-9a-wy-z](?:-[a-z0-9]{2,8})+)*(?:-x(?:-[a-z0-9]{1,8})+)?$/.exec(
      tag,
    );
  if (!match) return undefined;
  if (match[1] === "en") return "en";
  const [, , script, region] = match;
  if (script) return script === "hant" ? "zh-TW" : "zh-CN";
  return region === "tw" || region === "hk" || region === "mo"
    ? "zh-TW"
    : "zh-CN";
}

export function initialLanguage(
  saved: string | null,
  locales: readonly string[],
): Language {
  return (
    resolveLanguage(saved) ?? locales.map(resolveLanguage).find(Boolean) ?? "en"
  );
}
