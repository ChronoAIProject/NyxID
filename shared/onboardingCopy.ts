import simplified from "./i18n/onboarding/zh-CN.json";
import traditional from "./i18n/onboarding/zh-TW.json";

/** Exact server-authored onboarding copy. Unknown copy retains the server's
 * wording. IDs, provider names, user messages and approval decisions stay data. */
export function onboardingText(language: string | undefined, source: string): string {
  const catalog: Record<string, string> | undefined =
    language === "zh-CN" ? simplified : language === "zh-TW" ? traditional : undefined;
  return catalog && Object.prototype.hasOwnProperty.call(catalog, source) ? catalog[source]! : source;
}
