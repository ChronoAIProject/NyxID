// The template check's report, copied next to this file as
// `skill-template-report.local.json` (git-ignored). It stays out of `public/`
// and is reachable only from the dev preview pages, which are not production
// build entries, so a local report never ships in a build.
const REPORT = "./skill-template-report.local.json";
const reports = import.meta.glob<{ readonly default: unknown }>("./skill-template-report.local.json");

export const MISSING_REPORT =
  "No report found. Run the template check with NYX_SKILL_TEMPLATE_REPORT, then copy it to frontend/src/dev/skill-template-report.local.json.";

export async function loadSkillTemplateReport<T>(): Promise<T> {
  const load = reports[REPORT];
  if (!load) throw new Error(MISSING_REPORT);
  return (await load()).default as T;
}
