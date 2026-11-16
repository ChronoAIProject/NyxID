const LANGUAGE_ALIASES: Record<string, string> = {
  cmn: "zh",
  iw: "he",
  in: "id",
  ji: "yi",
  no: "nb",
  tl: "fil",
};

type Tag = { language: string; script?: string; region?: string };

function parse(value: string): Tag {
  const [language = "", ...rest] = value.replace(/_/g, "-").toLowerCase().split("-");
  return {
    language: LANGUAGE_ALIASES[language] ?? language,
    script: rest.find((part) => /^[a-z]{4}$/.test(part)),
    region: rest.find((part) => /^([a-z]{2}|\d{3})$/.test(part)),
  };
}

/** Saved choices may use a recognizer alias, but never silently change region. */
export function supportedDictationLocale(
  preferred: string,
  supported: readonly string[],
) {
  const wanted = parse(preferred);
  return supported.find((value) => {
    const candidate = parse(value);
    return (
      (candidate.language === wanted.language ||
        (["hk", "mo"].includes(wanted.region ?? "") &&
          [candidate.language, wanted.language].every((language) =>
            ["zh", "yue"].includes(language),
          ))) &&
      candidate.region === wanted.region &&
      (!candidate.script || !wanted.script || candidate.script === wanted.script)
    );
  });
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
  if (
    exact &&
    !(wanted.language === "zh" && ["hk", "mo"].includes(wanted.region ?? ""))
  ) return exact.value;
  // Apple's zh-HK recognizer is Cantonese; Android spells it yue.
  if (
    ["zh", "yue"].includes(wanted.language) &&
    ["hk", "mo"].includes(wanted.region ?? "")
  ) {
    const cantonese = candidates.find(
      ({ tag }) =>
        (tag.language === "yue" || tag.language === "zh") &&
        tag.region === wanted.region,
    );
    const yue = candidates.find(
      ({ tag }) => tag.language === "yue" && tag.region === wanted.region,
    );
    if (yue || cantonese) return (yue ?? cantonese)!.value;
    wanted.language = "zh";
    wanted.script = "hant";
    return candidates.find(
      ({ tag }) => tag.language === "zh" &&
        (tag.script === "hant" || ["tw", "hk", "mo"].includes(tag.region ?? "")),
    )?.value;
  }
  const language = candidates.filter(
    ({ tag }) => tag.language === wanted.language &&
      (!tag.script || !wanted.script || tag.script === wanted.script),
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
