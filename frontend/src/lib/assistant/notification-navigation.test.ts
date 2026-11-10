import { describe, expect, it } from "vitest";
import { notificationTarget, notificationTargetFromUrl } from "./notification-navigation";

const ORIGIN = "https://nyxid.example";
const source = { type: "conversation" as const, id: "nyxa-1", label: "", conversation_id: null, group_id: null, channel_id: null, trigger_id: null, agent_id: null };

describe("notificationTargetFromUrl", () => {
  it("turns assistant page links into in-app routes with only the keys those routes own", () => {
    expect(notificationTargetFromUrl(`${ORIGIN}/assistant?c=nyxa-1&card=abc&token=secret`, ORIGIN)).toEqual({
      kind: "route", to: "/assistant", search: { c: "nyxa-1", card: "abc" },
    });
    expect(notificationTargetFromUrl("/assistant?g=nyxg-1", ORIGIN)).toEqual({ kind: "route", to: "/assistant", search: { g: "nyxg-1" } });
    expect(notificationTargetFromUrl(`${ORIGIN}/assistant/automations/?agent=agent-1`, ORIGIN)).toEqual({
      kind: "route", to: "/assistant/automations", search: { agent: "agent-1" },
    });
    expect(notificationTargetFromUrl(`${ORIGIN}/assistant/approvals`, ORIGIN)).toEqual({ kind: "route", to: "/assistant/approvals", search: {} });
  });

  it("keeps other same-origin pages and foreign links as plain links, and refuses credentials", () => {
    expect(notificationTargetFromUrl(`${ORIGIN}/channel-bots/abc`, ORIGIN)).toEqual({ kind: "link", href: `${ORIGIN}/channel-bots/abc` });
    expect(notificationTargetFromUrl("https://evil.example/assistant?c=nyxa-1", ORIGIN)).toEqual({ kind: "link", href: "https://evil.example/assistant?c=nyxa-1" });
    expect(notificationTargetFromUrl("https://user:pw@nyxid.example/assistant?c=nyxa-1", ORIGIN)).toBeNull();
    expect(notificationTargetFromUrl("http://[", ORIGIN)).toBeNull();
  });
});

describe("notificationTarget", () => {
  it("prefers the server link and falls back to the row's source", () => {
    expect(notificationTarget({ url: `${ORIGIN}/assistant?c=nyxa-2`, source }, ORIGIN)).toEqual({ kind: "route", to: "/assistant", search: { c: "nyxa-2" } });
    expect(notificationTarget({ url: "", source: { ...source, type: "channel_chat", id: "thread-1", conversation_id: "nyxa-3" } }, ORIGIN)).toEqual({ kind: "route", to: "/assistant", search: { c: "nyxa-3" } });
    expect(notificationTarget({ url: "", source }, ORIGIN)).toEqual({ kind: "route", to: "/assistant", search: { c: "nyxa-1" } });
    expect(notificationTarget({ url: "", source: { ...source, type: "collaboration_group", id: "nyxg-2" } }, ORIGIN)).toEqual({ kind: "route", to: "/assistant", search: { g: "nyxg-2" } });
    expect(notificationTarget({ url: "", source: { ...source, type: "trigger", id: "trg-1", agent_id: "agent-1" } }, ORIGIN)).toEqual({
      kind: "route", to: "/assistant/automations", search: { agent: "agent-1" },
    });
    expect(notificationTarget({ url: "", source: { ...source, type: "channel_chat", id: "nyxa-4" } }, ORIGIN)).toEqual({ kind: "route", to: "/assistant", search: { c: "nyxa-4" } });
    expect(notificationTarget({ url: "", source: { ...source, type: "group", id: "" } }, ORIGIN)).toEqual({ kind: "route", to: "/assistant", search: { g: "" } });
  });
});
