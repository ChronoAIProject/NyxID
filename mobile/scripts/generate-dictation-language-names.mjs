import { writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

// Bounded reference vocabulary, never an availability list. The picker is always
// intersected with getSupportedLocales() from the installed recognition service.
// iOS: SFSpeechRecognizer.supportedLocales(), Apple's Speech framework locale
// identifiers (https://developer.apple.com/documentation/speech/sfspeechrecognizer/supportedlocales()).
export const iosLocales =
  `ar-SA ca-ES cs-CZ da-DK de-AT de-CH de-DE el-GR en-AE en-AU en-CA en-GB en-ID en-IE en-IN en-NZ en-PH en-SA en-SG en-US en-ZA es-AR es-CL es-CO es-ES es-MX es-US fi-FI fr-BE fr-CA fr-CH fr-FR he-IL hi-IN hr-HR hu-HU id-ID it-CH it-IT ja-JP ko-KR ms-MY nb-NO nl-BE nl-NL pl-PL pt-BR pt-PT ro-RO ru-RU sk-SK sv-SE th-TH tr-TR uk-UA vi-VN yue-CN yue-HK zh-CN zh-HK zh-TW`.split(
    " ",
  );
// Android recognition services have no platform-wide fixed locale set. This
// reference list is Google's voice-search language vocabulary, documented at
// https://cloud.google.com/speech-to-text/docs/speech-to-text-supported-languages
// (BCP-47 codes shared with Google's Android recognizer). Vendor additions use
// the language-only fallback; no list here claims installed/on-device support.
export const androidLocales =
  `af-ZA am-ET ar-AE ar-BH ar-DZ ar-EG ar-IL ar-IQ ar-JO ar-KW ar-LB ar-MA ar-OM ar-PS ar-QA ar-SA ar-TN az-AZ bg-BG bn-BD bn-IN bs-BA ca-ES cs-CZ da-DK de-DE el-GR en-AU en-CA en-GB en-GH en-HK en-IE en-IN en-KE en-NG en-NZ en-PH en-PK en-SG en-TZ en-US en-ZA es-AR es-BO es-CL es-CO es-CR es-DO es-EC es-ES es-GT es-HN es-MX es-NI es-PA es-PE es-PR es-PY es-SV es-US es-UY es-VE et-EE eu-ES fa-IR fi-FI fil-PH fr-CA fr-FR gl-ES gu-IN he-IL hi-IN hr-HR hu-HU hy-AM id-ID is-IS it-IT ja-JP jv-ID ka-GE kk-KZ km-KH kn-IN ko-KR lo-LA lt-LT lv-LV mk-MK ml-IN mn-MN mr-IN ms-MY my-MM ne-NP nl-BE nl-NL no-NO pa-Guru-IN pl-PL pt-BR pt-PT ro-RO ru-RU si-LK sk-SK sl-SI sq-AL sr-RS su-ID sv-SE sw-KE sw-TZ ta-IN ta-LK ta-MY ta-SG te-IN th-TH tr-TR uk-UA ur-IN ur-PK uz-UZ vi-VN yue-Hant-HK zh-CN zh-HK zh-TW zu-ZA`.split(
    " ",
  );
export const locales = [...new Set([...iosLocales, ...androidLocales])].sort();
export function entry(locale) {
  const tag = new Intl.Locale(locale);
  const languages = ["en", "zh-Hans", "zh-Hant"];
  const names = languages.map((ui) => {
    const language = new Intl.DisplayNames([ui], { type: "language" }).of(
      tag.language,
    );
    const region = tag.region
      ? new Intl.DisplayNames([ui], { type: "region" }).of(tag.region)
      : "";
    return [language, region];
  });
  return [
    ...names,
    new Intl.DisplayNames([locale], { type: "language" }).of(tag.language),
  ];
}
export function generate() {
  const data = Object.fromEntries(
    locales.map((locale) => [locale, entry(locale)]),
  );
  writeFileSync(
    fileURLToPath(
      new URL(
        "../src/features/assistant/voice/dictationLanguageNames.json",
        import.meta.url,
      ),
    ),
    `${JSON.stringify(data, null, 2)}\n`,
  );
}
if (process.argv[1] === fileURLToPath(import.meta.url)) generate();
