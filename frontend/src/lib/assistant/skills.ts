// Mock skills catalog for the assistant Plugins marketplace (Skills tab).
// Entries mirror the real Ornn registry skills (ornn repo, skills/*/SKILL.md).
// TODO(api-pass): this maps onto the Ornn skill-lifecycle API, reached through
// the NyxID proxy (`/api/v1/proxy/s/ornn-api/...`) — catalog entries come from
// Ornn skill search/list, `version`/`author` from the registry's version and
// ownership records, and `added` from the per-user install state
// (`~/.ornn/installed-skills.json` equivalent, server-side).

export interface SkillCatalogItem {
  id: string;
  name: string;
  initial: string;
  description: string;
  author: string;
  version: string;
  added: boolean;
}

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

export const SKILL_CATALOG: readonly SkillCatalogItem[] = [
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
  },
];
