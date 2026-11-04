type Tag = { language: string; script?: string; region?: string };

function parse(value: string): Tag {
  const [language = "", ...rest] = value.replace(/_/g, "-").toLowerCase().split("-");
  return {
    language,
    script: rest.find((part) => /^[a-z]{4}$/.test(part)),
    region: rest.find((part) => /^([a-z]{2}|\d{3})$/.test(part)),
  };
}

// Chinese recognizers are keyed by region, so a script-only phone language
// ("zh-Hant") needs the region that writes it.
const CHINESE_SCRIPT_REGIONS: Record<string, readonly string[]> = {
  hans: ["cn", "sg"],
  hant: ["tw", "hk", "mo"],
};

/**
 * The installed recognizer locale closest to the phone's language: exact,
 * then the same language and region (ignoring script), then the same language
 * in its home region. Undefined lets the recognizer use its own default.
 */
export function dictationLocale(
  preferred: string | undefined,
  supported: readonly string[],
): string | undefined {
  if (!preferred) return undefined;
  const wanted = parse(preferred);
  if (!wanted.language) return undefined;
  const candidates = supported.map((value) => ({ value, tag: parse(value) }));
  const normalized = preferred.replace(/_/g, "-").toLowerCase();
  const exact = candidates.find(
    ({ value }) => value.replace(/_/g, "-").toLowerCase() === normalized,
  );
  if (exact) return exact.value;
  const language = candidates.filter(
    ({ tag }) => tag.language === wanted.language,
  );
  const inRegion = (region?: string) =>
    region ? language.find(({ tag }) => tag.region === region) : undefined;
  const scriptRegions = wanted.script
    ? (CHINESE_SCRIPT_REGIONS[wanted.script] ?? [])
    : [];
  // A Chinese region that writes the other script ("zh-Hant-CN") loses to the script.
  const ownRegion =
    !scriptRegions.length || scriptRegions.includes(wanted.region ?? "");
  return (
    (ownRegion ? inRegion(wanted.region) : undefined) ??
    scriptRegions.map(inRegion).find(Boolean) ??
    inRegion(wanted.language) ??
    (wanted.language === "en" ? inRegion("us") : undefined) ??
    language[0]
  )?.value;
}
