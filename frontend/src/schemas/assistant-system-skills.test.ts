import { expect, it } from "vitest";
import { systemSkillsSelectionSchema } from "./assistant-system-skills";
import { agentSkillsSchema } from "./agent-skills";
it("requires exact versions, bounds the set and rejects supplied hashes", () => {
  const entry = { skill: "nyxbot", version: "1.0" };
  expect(
    systemSkillsSelectionSchema.safeParse({
      expected_revision: 0,
      skills: [entry],
    }).success,
  ).toBe(true);
  expect(
    systemSkillsSelectionSchema.safeParse({ expected_revision: 1, skills: [] })
      .success,
  ).toBe(true);
  for (const skills of [
    [{ ...entry, version: "latest" }],
    [{ ...entry, sha256: "client-hash" }],
    Array(9).fill(entry),
  ]) {
    expect(
      systemSkillsSelectionSchema.safeParse({ expected_revision: 0, skills })
        .success,
    ).toBe(false);
  }
});
it("keeps additive built-in statuses and accepts older responses", () => {
  const baseline = { revision: 0, skills: [], metadata: {} };
  expect(agentSkillsSchema.parse(baseline).system_skills).toBeUndefined();
  const status = {
    skill_id: "id",
    name: "nyxbot",
    version: "1.0",
    description: "Guide",
    enabled: false,
    effective: false,
    override: false,
    source: "pin",
    shadowed_by_pin: true,
  };
  expect(
    agentSkillsSchema.parse({ ...baseline, system_skills: [status] })
      .system_skills,
  ).toEqual([status]);
});
