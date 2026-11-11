import { expect, it } from "vitest";
import {
  legacyTelegramChat,
  personalChatSchema,
  personalLinkPreviewSchema,
  personalLinkStatusSchema,
} from "./platform-personal";
import {
  nyxAgentConversationSchema,
  nyxAgentHistorySchema,
} from "./assistant-nyxagent";

function nativeChat(platform: "slack" | "discord") {
  const prefix = platform === "discord" ? "/nyx " : "";
  return {
    platform,
    display_name: platform === "discord" ? "Discord" : "Slack",
    login_command: `${prefix}login`,
    logout_command: `${prefix}logout`,
    status_command: `${prefix}status`,
    help_command: `${prefix}help`,
    question_instruction: platform === "discord"
      ? 'Use /nyx ask question:"your question" in this app’s private chat.'
      : "Send ordinary text in this bot's private chat.",
    transport_disclosure: `This ${platform} chat is operated by the platform.`,
  };
}

const binding = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
const expires = "2026-10-18T00:00:00Z";

it.each(["slack", "discord"] as const)("accepts opaque %s identities and only its supported controls", (platform) => {
  const chat = nativeChat(platform);
  expect(personalChatSchema.parse(chat)).toEqual(chat);
  expect(personalChatSchema.safeParse({ ...chat, login_command: "/login" }).success).toBe(false);
  const preview = {
    chat,
    purpose: "personal_nyxbot",
    status: "pending",
    manager: { id: "manager", username: null, display_name: "Main bot" },
    sender: { id: "U-native-person", display_name: null },
    expires_at: expires,
    invitation_required: true,
    requires_same_chat_confirmation: true,
  };
  expect(personalLinkPreviewSchema.parse(preview).sender.id).toBe("U-native-person");
  for (const id of [" padded", "line\nbreak", "", "id\u0081value", "😀".repeat(65), "界".repeat(86)]) {
    expect(personalLinkPreviewSchema.safeParse({ ...preview, sender: { ...preview.sender, id } }).success).toBe(false);
  }
  for (const id of ["😀".repeat(64), "界".repeat(85) + "a"]) {
    expect(personalLinkPreviewSchema.safeParse({ ...preview, sender: { ...preview.sender, id } }).success).toBe(true);
  }
  const command = platform === "discord" ? "/nyx confirm code:P-ABCD-EFGH" : "confirm P-ABCD-EFGH";
  const status = { chat, challenge_id: binding, status: "awaiting_chat_confirmation", expires_at: expires, binding: null, confirmation_command: command };
  expect(personalLinkStatusSchema.parse(status).confirmation_command).toBe(command);
  expect(personalLinkStatusSchema.safeParse({ ...status, confirmation_command: "/confirm P-ABCD-EFGH" }).success).toBe(false);
});

it("defaults only a successful old response without additive chat metadata to Telegram", () => {
  const status = { challenge_id: binding, status: "confirmed", expires_at: expires, binding: { id: binding, generation: 1 }, confirmation_command: null };
  expect(personalLinkStatusSchema.parse(status).chat).toEqual(legacyTelegramChat);
  expect(personalLinkStatusSchema.safeParse({ ...status, chat: null }).success).toBe(false);
});

it.each(["slack", "discord", "whatsapp", "lark", "feishu", "x", "telegram"])("retains a saved %s thread through list/detail/history decoding", (platform) => {
  const thread = { platform, manager_id: "manager", binding_id: binding, binding_generation: 2 };
  const conversation = {
    id: `nyxa-${"a".repeat(32)}`, title: "Saved private work", model: "nyxagent/chat",
    created_at: expires, last_message_at: expires, message_count: 2, active_turn: null,
    context_reset_at: null, platform_thread: thread,
  };
  const origin = { binding_id: binding, binding_generation: 2, manager_authority_epoch: 3, personal_control_revision: 4 };
  expect(nyxAgentConversationSchema.parse(conversation).platform_thread).toEqual(thread);
  const history = nyxAgentHistorySchema.parse({
    conversation,
    before_seq: null,
    messages: [{ id: "saved-reply", seq: 2, turn_id: "turn", role: "assistant", text: "Continue in the browser", status: "completed", error_code: null, created_at: expires, platform_origin: origin }],
  });
  expect(history.conversation.platform_thread).toEqual(thread);
  expect(history.messages[0]?.platform_origin).toEqual(origin);
});
