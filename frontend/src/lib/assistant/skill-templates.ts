import packages from "../../../../shared/nyxbot-skill-templates.json";
import { translateUi } from "@/i18n/use-ui-translation";

// Published Ornn roots verified for this gallery. Updating a package is an
// explicit change to all three pin fields, never a lookup of the latest tag.
export const SKILL_TEMPLATES = [
  {
    slug: "research-paper-writing-rigor",
    ...packages["research-paper-writing-rigor"],
    category: "research",
    title: () => translateUi("Strengthen a research paper"),
    description: () => translateUi("Review a draft for reproducible methods, complete reporting, and conclusions that match the evidence."),
    inputs: () => [
      [translateUi("Manuscript"), translateUi("Share the draft, outline, or sections you want reviewed.")],
      [translateUi("Contribution"), translateUi("State the main result and the evidence or proof behind it.")],
      [translateUi("Scope"), translateUi("Name the target venue and where you want the review to focus.")],
    ] as const,
    output: () => translateUi("Prioritized revision checklist with gaps in the argument, evidence, and reproducibility"),
  },
  {
    slug: "research-claim-fidelity-reviewer",
    ...packages["research-claim-fidelity-reviewer"],
    category: "research",
    title: () => translateUi("Audit research claims"),
    description: () => translateUi("Check each claim against its source and flag wording that overstates the findings."),
    inputs: () => [
      [translateUi("Draft copy"), translateUi("Paste the post or thread and describe its intended audience.")],
      [translateUi("Source material"), translateUi("Share the paper, artifact links, or excerpts that support the claims.")],
      [translateUi("Claims to check"), translateUi("Flag the numbers or technical statements that need close review.")],
    ] as const,
    output: () => translateUi("Claim-by-claim verdicts, source anchors, and safer wording"),
  },
  {
    slug: "source-grounded-research-announcement",
    ...packages["source-grounded-research-announcement"],
    category: "communications",
    title: () => translateUi("Announce a research result"),
    description: () => translateUi("Turn a paper and supporting materials into a readable announcement with source-backed claims."),
    inputs: () => [
      [translateUi("Research artifact"), translateUi("Share the paper and the links readers can use to verify it.")],
      [translateUi("Claim ledger"), translateUi("List each proposed claim, its exact source, and any caveats.")],
      [translateUi("Announcement"), translateUi("Choose the audience and format, such as a short post or X thread.")],
    ] as const,
    output: () => translateUi("Draft announcement, claim ledger, and reviewer checklist"),
  },
  {
    slug: "public-copy-release-guard",
    ...packages["public-copy-release-guard"],
    category: "communications",
    title: () => translateUi("Review copy before publishing"),
    description: () => translateUi("Check public copy for sensitive details, unapproved claims, and wording that needs review."),
    inputs: () => [
      [translateUi("Copy to review"), translateUi("Paste the text you plan to publish and name the channel.")],
      [translateUi("Claims and sources"), translateUi("Share the approved claims and their supporting evidence.")],
      [translateUi("Release constraints"), translateUi("List any internal terms, topic limits, or details to keep private.")],
    ] as const,
    output: () => translateUi("Release blockers with suggested edits and a final pass checklist"),
  },
  {
    slug: "weekly-budget-variance-digest",
    ...packages["weekly-budget-variance-digest"],
    category: "operations",
    title: () => translateUi("Explain weekly budget changes"),
    description: () => translateUi("Compare actual spend with budget, highlight material differences, and surface follow-up questions."),
    inputs: () => [
      [translateUi("Reporting period"), translateUi("Specify the week and the teams or accounts to include.")],
      [translateUi("Budget and actuals"), translateUi("Share planned and actual spend for each category.")],
      [translateUi("Material variances"), translateUi("Specify the threshold and include any known explanations.")],
    ] as const,
    output: () => translateUi("Variance table, likely explanations, and follow-up questions"),
  },
  {
    slug: "saas-license-utilization-review",
    ...packages["saas-license-utilization-review"],
    category: "operations",
    title: () => translateUi("Review software license usage"),
    description: () => translateUi("Find unused seats in a license export and estimate savings for the next renewal."),
    inputs: () => [
      [translateUi("License inventory"), translateUi("Share the products, plans, and assigned seat counts.")],
      [translateUi("Usage data"), translateUi("Provide activity by seat and the period it covers.")],
      [translateUi("Renewals"), translateUi("Include per-seat costs and upcoming renewal dates.")],
    ] as const,
    output: () => translateUi("Unused-seat shortlist, estimated savings, and renewal questions"),
  },
  {
    slug: "release-readiness-review",
    ...packages["release-readiness-review"],
    category: "engineering",
    title: () => translateUi("Assess release readiness"),
    description: () => translateUi("Summarize release evidence, open blockers, and rollback coverage for a go/no-go review."),
    inputs: () => [
      [translateUi("Release scope"), translateUi("Describe the changes and list unresolved issues.")],
      [translateUi("Readiness evidence"), translateUi("Share test results, recent backups, and monitoring coverage.")],
      [translateUi("Rollback"), translateUi("Provide the rollback plan and the conditions for using it.")],
    ] as const,
    output: () => translateUi("Go/no-go recommendation with blockers, missing evidence, and rollback checks"),
  },
] as const;

export type SkillTemplate = (typeof SKILL_TEMPLATES)[number];

export function findSkillTemplate(slug: unknown): SkillTemplate | undefined {
  return SKILL_TEMPLATES.find((template) => template.slug === slug);
}

export function parseSkillGallerySearch(search: Record<string, unknown>): {
  skill?: SkillTemplate["slug"];
  code?: string;
} {
  const template = findSkillTemplate(search.skill);
  const code = search.code;
  return {
    ...(template ? { skill: template.slug } : {}),
    ...(typeof code === "string" && code.length > 0 && code.length <= 64 ? { code } : {}),
  };
}
