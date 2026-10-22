import { expect, it } from "vitest";
import {
  assistantGuestPolicySchema,
  assistantGuestPolicyPatchSchema,
  assistantGuestAccessViewSchema,
} from "./assistant-nyxagent";

it("accepts partial policy writes and clearing named overrides", () => {
  expect(
    assistantGuestPolicyPatchSchema.parse({
      services: { github: "default", slack: "all" },
    }),
  ).toEqual({ services: { github: "default", slack: "all" } });
  expect(
    assistantGuestPolicySchema.safeParse({
      default_access: "all",
      escalation: "ask_owner",
      share_skills: true,
    }).success,
  ).toBe(false);
  expect(
    assistantGuestPolicyPatchSchema.safeParse({ default_access: "all" })
      .success,
  ).toBe(false);
});

it("validates effective levels and their source", () => {
  expect(
    assistantGuestAccessViewSchema.parse({
      agent_id: "specialist",
      guest_policy: {
        enabled: false,
        default_access: "read",
        escalation: "refuse",
        share_skills: false,
      },
      effective_guest_policy: {
        enabled: false,
        default_access: "use",
        escalation: "refuse",
        share_skills: false,
      },
      services: { github: { access: "use", source: "explicit" } },
    }).services.github?.source,
  ).toBe("explicit");
});
