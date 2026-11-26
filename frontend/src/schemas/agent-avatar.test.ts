import { describe, expect, it } from "vitest";
import { NYXBOT_AVATAR } from "@/lib/assistant/agent-avatar-model";
import {
  agentAvatarFieldSchema,
  agentAvatarFormSchema,
  avatarPatch,
} from "./agent-avatar";
import {
  assistantAgentSchema,
  assistantGroupMemberSchema,
  nyxAgentConversationAgentSchema,
} from "./assistant-nyxagent";
import { notificationActorSchema } from "./assistant-notifications";

describe("agent avatar schemas", () => {
  it("reads ids a newer release wrote, and treats junk as no avatar", () => {
    expect(
      agentAvatarFieldSchema.parse({
        template: "face-v9",
        choices: { eyes: "comet", future_key: "future-value" },
      }),
    ).toMatchObject({ choices: { eyes: "comet", future_key: "future-value" } });
    expect(agentAvatarFieldSchema.parse({ body: 3 })).toBeNull();
    expect(agentAvatarFieldSchema.parse(undefined)).toBeUndefined();
    expect(agentAvatarFieldSchema.parse(null)).toBeNull();
  });

  it("is optional on every agent DTO", () => {
    const base = { id: "a", name: "researcher", kind: "specialist" };
    expect(nyxAgentConversationAgentSchema.parse(base).avatar).toBeUndefined();
    expect(
      nyxAgentConversationAgentSchema.parse({ ...base, avatar: NYXBOT_AVATAR })
        .avatar,
    ).toEqual(NYXBOT_AVATAR);
    expect(
      assistantGroupMemberSchema.parse({ ...base, avatar: NYXBOT_AVATAR })
        .avatar,
    ).toEqual(NYXBOT_AVATAR);
    expect(
      notificationActorSchema.parse({
        type: "agent",
        name: "x",
        avatar: NYXBOT_AVATAR,
      }).avatar,
    ).toEqual(NYXBOT_AVATAR);
    expect(
      assistantAgentSchema.shape.avatar.parse({
        ...NYXBOT_AVATAR,
        choices: { eyes: 7 },
      }),
    ).toBeNull();
  });

  it("only submits complete catalogue configurations", () => {
    expect(
      agentAvatarFormSchema.safeParse({ avatar: NYXBOT_AVATAR }).success,
    ).toBe(true);
    expect(
      agentAvatarFormSchema.safeParse({
        avatar: {
          template: "face-v9",
          choices: { eyes: "comet", future_key: "future-value" },
        },
      }).success,
    ).toBe(false);
    expect(
      agentAvatarFormSchema.safeParse({
        avatar: { ...NYXBOT_AVATAR, choices: { palette: "#ff0000" } },
      }).success,
    ).toBe(false);
    expect(avatarPatch(null)).toEqual({ avatar: null });
    expect(avatarPatch(NYXBOT_AVATAR)).toEqual({ avatar: NYXBOT_AVATAR });
  });
});
