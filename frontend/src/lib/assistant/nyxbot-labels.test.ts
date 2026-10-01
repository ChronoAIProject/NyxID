import { describe, expect, it } from "vitest";
import {
  agentHandle,
  agentTitle,
  channelChatTitle,
  splitChannelThreads,
  describeEventNotice,
  eventNotices,
  greetingFor,
  groupWithDisplayNames,
  withDisplayName,
  workingLabel,
} from "./nyxbot-labels";

describe("eventNotices", () => {
  it("strips the server's event header, whatever its parenthetical says", () => {
    const current =
      "NyxID events (authored by NyxID; only a quoted owner message is a request from the user):";
    const older = "NyxID events (notices, not user instructions):";
    for (const header of [current, older]) {
      expect(eventNotices(`${header}\n- first notice\n- second notice`)).toEqual([
        "first notice",
        "second notice",
      ]);
    }
    expect(eventNotices("Plain notice")).toEqual(["Plain notice"]);
  });
});

/** Texts as the backend writes them (handlers/assistant_team.rs, handlers/nyxbot.rs). */
describe("describeEventNotice", () => {
  it("summarises a specialist's reply and keeps the excerpt as detail", () => {
    expect(
      describeEventNotice(
        "Specialist researcher replied. Reply excerpt: \"Found 3 issues. See #12.\" Read more with nyxid__read_subagent.",
      ),
    ).toEqual({
      kind: "reply",
      tone: "neutral",
      summary: "researcher replied",
      detail: "Found 3 issues. See #12.",
      agent: "researcher",
    });
    expect(
      describeEventNotice(
        'Specialist writer failed (session_busy). Reply excerpt: "" Read more with nyxid__read_subagent.',
      ),
    ).toMatchObject({ tone: "destructive", summary: "writer failed", detail: undefined });
    expect(
      describeEventNotice(
        'Specialist writer was stopped. Reply excerpt: "half" Read more with nyxid__read_subagent.',
      ),
    ).toMatchObject({ tone: "warning", summary: "writer was stopped" });
  });

  it("summarises permission requests without the model-facing instruction", () => {
    const notice = describeEventNotice(
      'Specialist researcher requests service slack (request_id r-1). It was working on: "post the digest". Decide with nyxid__decide_permission: allow only what the user\'s request needs.',
    );
    expect(notice).toEqual({
      kind: "request",
      tone: "warning",
      summary: "researcher asks to use slack",
      detail: 'It was working on: "post the digest".',
      agent: "researcher",
    });
    expect(
      describeEventNotice(
        "Specialist researcher requests read-only account access (request_id r-2). It was working on: (no text) Decide with nyxid__decide_permission: x",
      ).summary,
    ).toBe("researcher asks to read your account");
  });

  it("summarises decisions, approvals, connections and channel bots", () => {
    expect(
      describeEventNotice(
        'NyxBot denied your request for service slack. Reason: "not needed". Do not retry it; finish what you can and report.',
      ),
    ).toMatchObject({
      kind: "decision",
      tone: "destructive",
      summary: "NyxBot denied slack",
      detail: "not needed",
    });
    expect(
      describeEventNotice(
        "The user allowed your request for service github. Retry the call now and continue your task.",
      ),
    ).toMatchObject({ tone: "success", summary: "You allowed github" });
    expect(
      describeEventNotice(
        "The user approved the github approval request (Create issue) via telegram. Retry that call now and continue; do not ask the user to confirm again.",
      ),
    ).toMatchObject({
      kind: "approval",
      summary: "You approved the github request via Telegram",
      detail: "Create issue",
    });
    expect(
      describeEventNotice("The user declined connecting notion (connect_link_id cl-9)."),
    ).toMatchObject({ kind: "connection", summary: "Declined connecting notion" });
    expect(
      describeEventNotice(
        "The telegram channel bot Home the user just created is now linked to NyxBot. Give the user its owner-verification step: https://t.me/x",
      ),
    ).toMatchObject({ kind: "channel", tone: "success", summary: "Telegram bot Home linked to NyxBot" });
    expect(
      describeEventNotice(
        "The user verified their telegram account on channel bot Home; it now reaches NyxBot there. No confirmation is needed.",
      ).summary,
    ).toBe("Verified your Telegram account on Home");
    expect(
      describeEventNotice(
        'The owner sent another telegram message while you were working. Answer it next; your reply is delivered to the chat: "and the weather?"',
      ),
    ).toMatchObject({ kind: "message", summary: "New Telegram message", detail: "and the weather?" });
  });

  it("falls back to the first sentence for unknown notices", () => {
    expect(describeEventNotice("Short.")).toEqual({
      kind: "notice",
      tone: "neutral",
      summary: "Short",
      detail: "Short.",
    });
    expect(describeEventNotice("No sentence end")).toMatchObject({
      summary: "No sentence end",
      detail: undefined,
    });
  });
});

describe("greetingFor", () => {
  it("greets by local hour", () => {
    expect(greetingFor(new Date(2026, 8, 29, 8))).toBe("Good morning");
    expect(greetingFor(new Date(2026, 8, 29, 13))).toBe("Good afternoon");
    expect(greetingFor(new Date(2026, 8, 29, 20))).toBe("Good evening");
    expect(greetingFor(new Date(2026, 8, 29, 2))).toBe("Good evening");
  });
});

describe("workingLabel", () => {
  it("names who is working", () => {
    expect(workingLabel(["researcher"])).toBe("researcher is working…");
    expect(workingLabel(["researcher", "NyxBot"])).toBe("researcher and NyxBot are working…");
    expect(workingLabel(["a", "b", "c", "d"])).toBe("a, b and 2 more are working…");
  });
});

describe("agent naming", () => {
  it("shows the display name with the @handle beside it", () => {
    const writer = { kind: "specialist" as const, name: "writer" };
    expect(agentTitle(writer)).toBe("writer");
    expect(agentHandle(writer)).toBeUndefined();
    expect(agentTitle({ ...writer, display_name: "Luna" })).toBe("Luna");
    expect(agentHandle({ ...writer, display_name: "Luna" })).toBe("@writer");
    expect(agentTitle({ ...writer, display_name: "  " })).toBe("writer");
    const nyxbot = { kind: "nyxbot" as const, name: "NyxBot" };
    expect(agentTitle(nyxbot)).toBe("NyxBot");
    expect(agentHandle(nyxbot)).toBeUndefined();
    expect(agentTitle({ ...nyxbot, display_name: "Nyx" })).toBe("Nyx");
    expect(agentHandle({ ...nyxbot, display_name: "Nyx" })).toBe("@NyxBot");
  });

  it("joins display names onto conversation and group refs by id", () => {
    const agents = [{ id: "a1", display_name: "Luna" }, { id: "a2", display_name: null }];
    expect(withDisplayName({ id: "a1", name: "writer" }, agents)).toEqual({
      id: "a1",
      name: "writer",
      display_name: "Luna",
    });
    expect(withDisplayName({ id: "a3", name: "gone" }, agents).display_name).toBeNull();
    expect(withDisplayName({ id: "a1", name: "writer" }, undefined).display_name).toBeNull();
    const group = groupWithDisplayNames(
      { name: "Crew", members: [{ id: "a1", name: "writer" }, { id: "a2", name: "x" }] },
      agents,
    );
    expect(group.members.map((member) => member.display_name)).toEqual(["Luna", null]);
  });
});

describe("channel chats", () => {
  const at = "2026-09-29T00:00:00.000Z";
  const thread = (id: string, channel?: Record<string, string | null>) => ({
    id,
    title: id,
    created_at: at,
    last_message_at: at,
    channel: channel ? { platform: "telegram", ...channel } : null,
  });

  it("splits an agent's own threads from each bot's chats, newest bot first", () => {
    const split = splitChannelThreads([
      thread("lark-group", { platform: "lark", channel_agent_id: "b", bot_label: "Office" }),
      thread("own"),
      thread("tg-private", { channel_agent_id: "a", bot_label: "Home" }),
      thread("lark-private", { platform: "lark", channel_agent_id: "b", bot_label: null }),
      thread("legacy", {}),
    ]);
    expect(split.own.map((row) => row.id)).toEqual(["own"]);
    expect(split.bots.map((group) => [group.key, group.label, group.threads.map((row) => row.id)])).toEqual([
      ["b", "Office", ["lark-group", "lark-private"]],
      ["a", "Home", ["tg-private"]],
      ["platform:telegram", "Telegram bot", ["legacy"]],
    ]);
  });

  it("names unnamed chats by kind and platform", () => {
    expect(channelChatTitle("group", "Team", "telegram")).toBe("Team");
    expect(channelChatTitle("private", null, "telegram")).toBe("Private Telegram chat");
    expect(channelChatTitle("group", null, "lark")).toBe("Lark group");
    expect(channelChatTitle("channel", null, "telegram")).toBe("Telegram channel");
    expect(channelChatTitle(null, null, "telegram")).toBe("Telegram chat");
  });
});
