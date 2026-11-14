import { expect, it } from "vitest";
import { agentSkillsSchema, systemSkillStatusSchema } from "./agent-skills";
import { nyxAgentSettingsSchema } from "./assistant-nyxagent";

it("keeps inheritance, effective state and mode in skill responses", () => {
  const status = {
    skill_id: "id",
    name: "Guide",
    version: "1.0",
    description: "Useful",
    enabled: true,
    effective: false,
    override: null,
    source: "pin",
    shadowed_by_pin: true,
  };
  const response = agentSkillsSchema.parse({
    revision: 0,
    skills: [],
    metadata: {},
    system_skills: [status],
    system_skills_for_specialists: "override",
  });
  expect(response.system_skills?.[0]).toEqual(status);
  expect(response.system_skills_for_specialists).toBe("override");
  for (const source of [
    "agent",
    "nyxbot_default",
    "nyxbot_override",
    "platform",
    "pin",
  ]) {
    expect(systemSkillStatusSchema.parse({ ...status, source }).source).toBe(
      source,
    );
  }
  expect(
    systemSkillStatusSchema.safeParse({ ...status, source: "unknown" }).success,
  ).toBe(false);
});
it("preserves omission of the specialist mode for ineligible and legacy responses", () => {
  const settings = nyxAgentSettingsSchema.parse({
    skip_destructive_confirmation: false,
    max_live_subagents: 8,
    max_concurrent_subagent_turns: 3,
    max_live_subagents_limit: 32,
    max_concurrent_subagent_turns_limit: 8,
  });
  expect(settings.system_skills_for_specialists).toBeUndefined();
});
