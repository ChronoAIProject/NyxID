import { ApiError } from "@/lib/api-client";
import {
  systemSkillsSchema,
  systemSkillsSelectionSchema,
  type SystemSkills,
} from "@/schemas/assistant-system-skills";

// Shared by the two DEV-only HTTP fixtures; no Ornn requests leave the browser.
const STORAGE = "nyxid.mock-system-skills-v1";
export const MOCK_PUBLIC_SYSTEM_SKILLS: SystemSkills["skills"] = [
  {
    skill_id: "b56b5bca-09d6-45f4-991a-05e945ac7eb4",
    name: "nyxbot",
    version: "0.1",
    sha256: "1".repeat(64),
    dependencies: [],
    description: "Coordinate tasks and work with your specialist team.",
    size_bytes: 2048,
  },
  {
    skill_id: "9b2b3798-6c7a-4ed9-9c15-43cfb92d342b",
    name: "nyxbot-ornn-skills",
    version: "0.1",
    sha256: "2".repeat(64),
    dependencies: [],
    description: "Find and read published Ornn skills when a task needs guidance.",
    size_bytes: 1536,
  },
  {
    skill_id: "959472fc-c4b5-4ede-8d6d-a5e5bf3ec749",
    name: "nyxbot-cma",
    version: "0.1",
    sha256: "3".repeat(64),
    dependencies: [],
    description: "Keep useful context and memory for ongoing work.",
    size_bytes: 1024,
  },
];

export function mockSystemSkillsConfig(): SystemSkills {
  const saved = sessionStorage.getItem(STORAGE);
  if (saved) {
    const parsed = systemSkillsSchema.safeParse(JSON.parse(saved));
    if (parsed.success) return parsed.data;
  }
  return systemSkillsSchema.parse({
    revision: 1,
    skills: MOCK_PUBLIC_SYSTEM_SKILLS,
    updated_by: "d4f5a6b7-c8d9-4e0f-a1b2-c3d4e5f60718",
    updated_at: "2026-11-10T08:00:00Z",
  });
}

function invalid(message: string): never {
  throw new ApiError(400, { error: "validation_error", error_code: 1008, message });
}

export function updateMockSystemSkills(body: unknown, actor: string): SystemSkills {
  const parsed = systemSkillsSelectionSchema.safeParse(body);
  if (!parsed.success) invalid("Choose up to 8 public skills at exact versions.");
  const current = mockSystemSkillsConfig();
  if (parsed.data.expected_revision !== current.revision) {
    throw new ApiError(409, {
      error: "conflict",
      error_code: 1004,
      message: "System skills changed; reload the current revision",
    });
  }
  const skills = parsed.data.skills.map((entry) => {
    const skill = MOCK_PUBLIC_SYSTEM_SKILLS.find(
      (item) => item.skill_id === entry.skill || item.name === entry.skill,
    );
    if (!skill || skill.version !== entry.version) invalid("Public skill version not found.");
    return skill;
  });
  if (new Set(skills.map((skill) => skill.skill_id)).size !== skills.length) {
    invalid("Duplicate system skill IDs or names");
  }
  const updated = systemSkillsSchema.parse({
    revision: current.revision + 1,
    skills,
    updated_by: actor,
    updated_at: new Date().toISOString(),
  });
  sessionStorage.setItem(STORAGE, JSON.stringify(updated));
  return updated;
}

export function mockSkillCatalog(params: URLSearchParams): unknown | undefined {
  const selected = params.get("skill");
  if (selected) {
    const skill = MOCK_PUBLIC_SYSTEM_SKILLS.find((item) => item.skill_id === selected);
    if (!skill) return undefined;
    if (params.has("version")) {
      if (params.get("version") !== skill.version) return undefined;
      const { description, size_bytes, ...reference } = skill;
      return { reference: { source: "ornn", ...reference, disabled: false }, description, size_bytes };
    }
    return { page: 1, total_pages: 1, items: [{ version: skill.version, sha256: skill.sha256, deprecated: false }] };
  }
  const query = (params.get("q") ?? "").toLowerCase();
  return {
    page: 1,
    total_pages: 1,
    items: MOCK_PUBLIC_SYSTEM_SKILLS.filter((skill) => `${skill.name} ${skill.description}`.toLowerCase().includes(query))
      .map((skill) => ({ id: skill.skill_id, name: skill.name, description: skill.description })),
  };
}

export function mockSkillMarkdown(name: string): string {
  return `---\nname: ${name}\ndescription: Local preview of a public built-in skill.\n---\n\n# ${name}\n\nThis placeholder SKILL.md is served by the local development fixture.\nUse this guidance to plan useful work. Skills never grant permissions.\n`;
}
