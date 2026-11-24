import { translateUi } from "@/i18n/use-ui-translation";

// Published Ornn roots verified for this gallery. Updating a package is an
// explicit change to all three pin fields, never a lookup of the latest tag.
export const SKILL_TEMPLATES = [
  {
    slug: "research-paper-writing-rigor",
    skillId: "a22344ff-4cf3-4ea7-a22e-2f0f98792c10",
    version: "2.0",
    sha256: "0820b8532ad9601d32749207469ebaaf15d66c49d5a077051f69a47ca845ad0e",
    category: "research",
    title: () => translateUi("Strengthen a research paper"),
    input: () => translateUi("Research draft and target venue"),
    description: () => translateUi("Review a draft for reproducible methods, complete reporting, and conclusions that match the evidence."),
    starter: () => translateUi("Ask me for a research draft and its target venue. Review the reporting, reproducibility, and strength of the conclusions. Return a prioritized revision checklist."),
  },
  {
    slug: "research-claim-fidelity-reviewer",
    skillId: "be585a18-7c28-45e3-a8f4-e098904e3304",
    version: "0.1",
    sha256: "585d20211e65ae72f52c7d02f5b3517c121a927b313f4a08a059b99a403dcf27",
    category: "research",
    title: () => translateUi("Audit research claims"),
    input: () => translateUi("Claims and source material"),
    description: () => translateUi("Check each claim against its source and flag wording that overstates the findings."),
    starter: () => translateUi("Ask me for the claims and their source material. Trace each claim to the evidence and suggest precise wording for anything unsupported or overstated."),
  },
  {
    slug: "source-grounded-research-announcement",
    skillId: "47cb35ba-4230-4a7b-88df-1377b8cf6896",
    version: "0.1",
    sha256: "d74e3a08785a9e144b891eca70bc562239305480e74104d2636234f95a51cd9e",
    category: "communications",
    title: () => translateUi("Announce a research result"),
    input: () => translateUi("Paper, audience, and announcement format"),
    description: () => translateUi("Turn a paper and supporting materials into a readable announcement with source-backed claims."),
    starter: () => translateUi("Ask me for the paper, intended audience, and announcement format. Draft copy in this chat that communicates the findings and their limits, with sources for the main claims."),
  },
  {
    slug: "public-copy-release-guard",
    skillId: "296dd304-8f24-4966-b0d9-7e1be2ee7c57",
    version: "0.2",
    sha256: "16a6b3eb91b35c20be92ac22f72fcbc46f929ac9a0a047072cef076c2b24575f",
    category: "communications",
    title: () => translateUi("Review copy before publishing"),
    input: () => translateUi("Draft copy, audience, and approved claims"),
    description: () => translateUi("Check public copy for sensitive details, unapproved claims, and wording that needs review."),
    starter: () => translateUi("Ask me for the proposed public copy, its audience, and any approved claims. Flag release blockers, explain each concern, and suggest edits in this chat."),
  },
  {
    slug: "weekly-budget-variance-digest",
    skillId: "e87baf06-c92d-440e-8858-63902175ba64",
    version: "1.1",
    sha256: "d36930f6546c6bd01bc0adcb86cf5d00e8f34bc8834ec7705e56cd46cf16b23d",
    category: "operations",
    title: () => translateUi("Explain weekly budget changes"),
    input: () => translateUi("Weekly budget, actual spend, and reporting threshold"),
    description: () => translateUi("Compare actual spend with budget, highlight material differences, and surface follow-up questions."),
    starter: () => translateUi("Ask me for this week’s budget, actual spend, and reporting threshold. Draft a budget variance digest in this chat and distinguish measured differences from possible causes."),
  },
  {
    slug: "saas-license-utilization-review",
    skillId: "0d1cfcd6-1a40-44cf-89fc-6159f7e5f44d",
    version: "2.2",
    sha256: "341afbb26924ea80807a1dedad804d48424e8e2e82124aeaa717dae46e61de7b",
    category: "operations",
    title: () => translateUi("Review software license usage"),
    input: () => translateUi("License inventory and usage export"),
    description: () => translateUi("Find unused seats in a license export and estimate savings for the next renewal."),
    starter: () => translateUi("Ask me for a license inventory and usage export. Identify possibly unused seats, estimate savings where costs are known, and summarize what to confirm before renewal."),
  },
  {
    slug: "release-readiness-review",
    skillId: "1069f6b4-ebb4-49f8-85fd-20c83c7b4036",
    version: "2.1",
    sha256: "2279c82293318a29e4c81f2b41e4e017b5bedaaffd81a1a5bb951f17ac74e7cc",
    category: "engineering",
    title: () => translateUi("Assess release readiness"),
    input: () => translateUi("Release scope, test results, open issues, and rollback plan"),
    description: () => translateUi("Summarize release evidence, open blockers, and rollback coverage for a go/no-go review."),
    starter: () => translateUi("Ask me for the release scope, test results, open issues, and rollback plan. Write a go/no-go recommendation and list the evidence still needed in this chat."),
  },
] as const;

export type SkillTemplate = (typeof SKILL_TEMPLATES)[number];

export function formatTemplateStarter(template: SkillTemplate): string {
  return translateUi("Use the Ornn skill {{skill}} (version {{version}}) for this task. {{starter}}", {
    skill: template.slug,
    version: template.version,
    starter: template.starter(),
  });
}

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
