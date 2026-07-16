import { beforeEach, describe, expect, it } from "vitest";
import {
  dedupeSkills,
  SKILL_CATALOG,
  addSkill,
  listAddedSkillIds,
  resetSkillCatalog,
  type SkillCatalogItem,
} from "./skills";

describe("SKILL_CATALOG", () => {
  it("has unique ids", () => {
    const ids = SKILL_CATALOG.map((item) => item.id);
    expect(new Set(ids).size).toBe(ids.length);
  });

  it("has unique names (no near-duplicate cards)", () => {
    const names = SKILL_CATALOG.map((item) => item.name);
    expect(new Set(names).size).toBe(names.length);
  });

  it("hides deprecated skills", () => {
    expect(SKILL_CATALOG.some((item) => item.deprecated)).toBe(false);
    // The deprecated HTTP manual is not shown.
    expect(SKILL_CATALOG.map((item) => item.id)).not.toContain(
      "ornn-agent-manual-http",
    );
  });

  it("has well-formed entries", () => {
    for (const item of SKILL_CATALOG) {
      expect(item.name).not.toBe("");
      expect(item.description).not.toBe("");
      expect(item.author).not.toBe("");
      expect(item.version).toMatch(/^v\d+\.\d+$/);
      expect(item.initial.length).toBeGreaterThanOrEqual(1);
      expect(item.initial.length).toBeLessThanOrEqual(2);
    }
  });

  it("seeds at least one skill as added", () => {
    expect(SKILL_CATALOG.some((item) => item.added)).toBe(true);
  });
});

describe("skill install state", () => {
  beforeEach(() => {
    resetSkillCatalog();
  });

  it("starts from the seeded added flags", () => {
    const seeded = SKILL_CATALOG.filter((item) => item.added).map(
      (item) => item.id,
    );
    expect([...listAddedSkillIds()].sort()).toEqual(seeded.sort());
  });

  it("addSkill adds an id and returns the new set", () => {
    const next = addSkill("ornn-agent-manual-cli");
    expect(next.has("ornn-agent-manual-cli")).toBe(true);
    expect(listAddedSkillIds().has("ornn-agent-manual-cli")).toBe(true);
  });

  it("addSkill keeps previously added ids", () => {
    addSkill("ornn-agent-manual-cli");
    const next = addSkill("chrono-ai-service-manual");
    for (const item of SKILL_CATALOG.filter((entry) => entry.added)) {
      expect(next.has(item.id)).toBe(true);
    }
    expect(next.has("ornn-agent-manual-cli")).toBe(true);
    expect(next.has("chrono-ai-service-manual")).toBe(true);
  });

  it("addSkill is idempotent", () => {
    const first = addSkill("ornn-agent-manual-cli");
    const second = addSkill("ornn-agent-manual-cli");
    expect(second.size).toBe(first.size);
  });

  it("resetSkillCatalog restores the seeded state", () => {
    addSkill("ornn-agent-manual-cli");
    resetSkillCatalog();
    expect(listAddedSkillIds().has("ornn-agent-manual-cli")).toBe(false);
    const seeded = SKILL_CATALOG.filter((item) => item.added).map(
      (item) => item.id,
    );
    expect([...listAddedSkillIds()].sort()).toEqual(seeded.sort());
  });
});

describe("dedupeSkills", () => {
  const base: SkillCatalogItem = {
    id: "s",
    name: "S",
    initial: "S",
    description: "d",
    author: "Ornn",
    version: "v1.0",
    added: false,
  };

  it("collapses a repeated id to the highest version", () => {
    const result = dedupeSkills([
      { ...base, id: "dup", version: "v1.0" },
      { ...base, id: "dup", version: "v1.3" },
      { ...base, id: "dup", version: "v1.1" },
    ]);
    expect(result).toHaveLength(1);
    expect(result[0]?.version).toBe("v1.3");
  });

  it("drops deprecated entries entirely", () => {
    const result = dedupeSkills([
      { ...base, id: "keep" },
      { ...base, id: "old", deprecated: true },
    ]);
    expect(result.map((s) => s.id)).toEqual(["keep"]);
  });
});
