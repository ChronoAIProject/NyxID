import { describe, expect, it } from "vitest";
import { agentSkillSchema } from "./agent-skills";

describe("agent skill schema", () => {
  it("defaults disabled pins to false and preserves true", () => {
    const base = { source: "ornn", skill_id: "s", name: "Skill", version: "1.0", sha256: "a".repeat(64) };
    expect(agentSkillSchema.parse(base).disabled).toBe(false);
    expect(agentSkillSchema.parse({ ...base, disabled: true }).disabled).toBe(true);
  });
});
