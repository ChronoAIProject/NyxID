// Mock skills catalog for the assistant Plugins marketplace (Skills tab).
// Entries mirror the real Ornn registry skills (ornn repo, skills/*/SKILL.md).
// TODO(api-pass): this maps onto the Ornn skill-lifecycle API, reached through
// the NyxID proxy (`/api/v1/proxy/s/ornn-api/...`) — catalog entries come from
// Ornn skill search/list, `version`/`author` from the registry's version and
// ownership records, `deprecated` from per-version `isDeprecated`, and `added`
// from the per-user install state (`~/.ornn/installed-skills.json` equivalent,
// server-side). Ornn search returns overlapping scopes (mine / public /
// shared-with-me) and multiple versions per skill, so the same skill can
// surface more than once — `dedupeSkills` guarantees one card per skill.

export interface SkillCatalogItem {
  id: string;
  name: string;
  initial: string;
  description: string;
  author: string;
  version: string;
  added: boolean;
  /** Superseded by a newer skill; hidden from the marketplace. */
  deprecated?: boolean;
}

/** Parse a `vMAJOR.MINOR` version into a comparable number. */
function versionRank(version: string): number {
  const match = /^v?(\d+)\.(\d+)$/.exec(version);
  if (!match) return 0;
  return Number(match[1]) * 1000 + Number(match[2]);
}

/**
 * One card per skill: drop deprecated (superseded) entries, then collapse any
 * repeated `id` to a single entry keeping the highest version. Mirrors the
 * connectors' one-card-per-service rule and pre-empts the API pass, where the
 * Ornn registry can return the same skill across scopes/versions.
 */
export function dedupeSkills(
  skills: readonly SkillCatalogItem[],
): readonly SkillCatalogItem[] {
  const byId = new Map<string, SkillCatalogItem>();
  for (const skill of skills) {
    if (skill.deprecated) continue;
    const existing = byId.get(skill.id);
    if (
      !existing ||
      versionRank(skill.version) > versionRank(existing.version)
    ) {
      byId.set(skill.id, skill);
    }
  }
  return [...byId.values()];
}

// Raw registry entries before de-duplication. The HTTP manual is deprecated in
// favor of the unified Chrono AI Service Manual, so it never renders.
const RAW_SKILLS: readonly SkillCatalogItem[] = [
  {
    id: "chrono-ai-service-manual",
    name: "Chrono AI Service Manual",
    initial: "CA",
    description:
      "Unified agent manual for the Chrono AI stack: NyxID identity, services, orgs, and proxy plus the full Ornn skill lifecycle in one install.",
    author: "Ornn",
    version: "v1.0",
    added: true,
  },
  {
    id: "ornn-agent-manual-cli",
    name: "Ornn Agent Manual (CLI)",
    initial: "OC",
    description:
      "Skill-lifecycle manual for agents driving Ornn through the NyxID CLI: search, pull, execute, build, upload, and share skills.",
    author: "Ornn",
    version: "v1.2",
    added: false,
  },
  {
    id: "ornn-agent-manual-http",
    name: "Ornn Agent Manual (HTTP)",
    initial: "OH",
    description:
      "Skill-lifecycle manual for agents calling the Ornn API over direct HTTPS with a NyxID bearer token. Deprecated in favor of the unified manual.",
    author: "Ornn",
    version: "v1.1",
    added: false,
    deprecated: true,
  },
];

export const SKILL_CATALOG: readonly SkillCatalogItem[] =
  dedupeSkills(RAW_SKILLS);

// Session-scoped install state: survives view remounts (navigating away and
// back), resets on reload — same lifecycle as the assistant mock store and
// the plugins catalog in ./plugins.ts.
let addedIds: ReadonlySet<string> | null = null;

function seededAddedIds(): Set<string> {
  return new Set(
    SKILL_CATALOG.filter((item) => item.added).map((item) => item.id),
  );
}

export function listAddedSkillIds(): ReadonlySet<string> {
  addedIds ??= seededAddedIds();
  return addedIds;
}

export function addSkill(id: string): ReadonlySet<string> {
  addedIds = new Set([...listAddedSkillIds(), id]);
  return addedIds;
}

export function resetSkillCatalog(): void {
  addedIds = null;
}
