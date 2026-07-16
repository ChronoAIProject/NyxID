import { beforeEach, describe, expect, it } from "vitest";
import {
  SKILL_CATALOG,
  addSkill,
  listAddedSkillIds,
  resetSkillCatalog,
} from "./skills";

describe("SKILL_CATALOG", () => {
  it("has unique ids", () => {
    const ids = SKILL_CATALOG.map((item) => item.id);
    expect(new Set(ids).size).toBe(ids.length);
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
    const next = addSkill("ornn-agent-manual-http");
    for (const item of SKILL_CATALOG.filter((entry) => entry.added)) {
      expect(next.has(item.id)).toBe(true);
    }
    expect(next.has("ornn-agent-manual-cli")).toBe(true);
    expect(next.has("ornn-agent-manual-http")).toBe(true);
  });

  it("addSkill is idempotent", () => {
    const first = addSkill("ornn-agent-manual-cli");
    const second = addSkill("ornn-agent-manual-cli");
    expect(second.size).toBe(first.size);
  });

  it("resetSkillCatalog restores the seeded state", () => {
    addSkill("ornn-agent-manual-http");
    resetSkillCatalog();
    expect(listAddedSkillIds().has("ornn-agent-manual-http")).toBe(false);
    const seeded = SKILL_CATALOG.filter((item) => item.added).map(
      (item) => item.id,
    );
    expect([...listAddedSkillIds()].sort()).toEqual(seeded.sort());
  });
});
