import { afterEach, beforeEach, expect, it, vi } from "vitest";
import {
  getNyxAgentLiveStatus,
  listenNyxAgentLive,
  parseNyxAgentLiveEvent,
  resetNyxAgentLiveForTests,
  type NyxAgentLiveEvent,
} from "./nyxagent-live";
import { nyxAgentLiveRefresh } from "@/hooks/use-nyxagent-live";

const encoder = new TextEncoder();

function sse(frames: readonly object[], keepOpen = true) {
  const stream = new ReadableStream<Uint8Array>({
    start(controller) {
      for (const frame of frames) {
        controller.enqueue(encoder.encode(`event: x\ndata: ${JSON.stringify(frame)}\n\n`));
      }
      if (!keepOpen) controller.close();
    },
  });
  return new Response(stream, { headers: { "content-type": "text/event-stream" } });
}

let requests: string[];

beforeEach(() => {
  resetNyxAgentLiveForTests();
  requests = [];
});

afterEach(() => {
  resetNyxAgentLiveForTests();
  globalThis.__nyxidAssistantHttpMock = undefined;
  vi.useRealTimers();
});

it("parses only known frames and ignores anything else", () => {
  expect(parseNyxAgentLiveEvent('{"type":"conversation","id":"nyxa-1","group_id":null}')).toEqual({
    type: "conversation",
    id: "nyxa-1",
    group_id: null,
    turn_id: null,
    messages: 0,
  });
  expect(parseNyxAgentLiveEvent('{"type":"group","id":"nyxg-1"}')).toEqual({
    type: "group",
    id: "nyxg-1",
  });
  expect(parseNyxAgentLiveEvent('{"type":"unknown"}')).toBeUndefined();
  expect(parseNyxAgentLiveEvent("not json")).toBeUndefined();
});

it("shares one stream, delivers its events, and closes it with the last listener", async () => {
  globalThis.__nyxidAssistantHttpMock = ({ endpoint }) => {
    requests.push(endpoint);
    return sse([{ type: "ready" }, { type: "conversation", id: "nyxa-1", group_id: "nyxg-1" }]);
  };
  const first: NyxAgentLiveEvent[] = [];
  const second: NyxAgentLiveEvent[] = [];
  const stopFirst = listenNyxAgentLive((event) => first.push(event));
  const stopSecond = listenNyxAgentLive((event) => second.push(event));
  await vi.waitFor(() => expect(first).toHaveLength(2));
  expect(second).toEqual(first);
  expect(first[1]).toEqual({
    type: "conversation",
    id: "nyxa-1",
    group_id: "nyxg-1",
    turn_id: null,
    messages: 0,
  });
  expect(getNyxAgentLiveStatus()).toBe("live");
  expect(requests).toEqual(["/assistant/nyxagent/live"]);
  stopFirst();
  expect(getNyxAgentLiveStatus()).toBe("live");
  stopSecond();
  expect(getNyxAgentLiveStatus()).toBe("idle");
});

it("falls back to polling when the server has no live stream", async () => {
  globalThis.__nyxidAssistantHttpMock = ({ endpoint }) => {
    requests.push(endpoint);
    return new Response(JSON.stringify({ message: "Assistant route not found." }), { status: 404 });
  };
  const stop = listenNyxAgentLive(() => undefined);
  await vi.waitFor(() => expect(getNyxAgentLiveStatus()).toBe("unsupported"));
  await new Promise((resolve) => setTimeout(resolve, 50));
  expect(requests).toHaveLength(1);
  stop();
});

it("treats an answer that is not an event stream as unsupported", async () => {
  globalThis.__nyxidAssistantHttpMock = ({ endpoint }) => {
    requests.push(endpoint);
    return new Response(JSON.stringify({ conversations: [] }));
  };
  const stop = listenNyxAgentLive(() => undefined);
  await vi.waitFor(() => expect(getNyxAgentLiveStatus()).toBe("unsupported"));
  expect(requests).toHaveLength(1);
  stop();
});

it("backs off instead of spinning when the stream keeps closing before it is ready", async () => {
  globalThis.__nyxidAssistantHttpMock = ({ endpoint }) => {
    requests.push(endpoint);
    return sse([], false);
  };
  const stop = listenNyxAgentLive(() => undefined);
  await vi.waitFor(() => expect(requests.length).toBeGreaterThanOrEqual(1));
  await new Promise((resolve) => setTimeout(resolve, 400));
  // One attempt, then a 1 s back-off: no tight loop.
  expect(requests).toHaveLength(1);
  expect(getNyxAgentLiveStatus()).toBe("connecting");
  stop();
});

it("refreshes the open thread on every change but lists only when a turn or count changes", () => {
  const seen = new Map<string, string>();
  const change = (turn: string | null, messages: number) =>
    nyxAgentLiveRefresh(
      "u",
      { type: "conversation", id: "nyxa-1", group_id: null, turn_id: turn, messages },
      seen,
    );
  const threadAndAgents = [
    ["assistant", "nyxagent", "u", "threads"],
    ["assistant", "nyxagent", "u", "agents"],
  ];
  // A turn starts: the thread and the lists.
  expect(change("t1", 2)).toEqual({
    now: [["assistant", "nyxagent", "u", "history", "nyxa-1"]],
    lists: threadAndAgents,
  });
  // Tool activity during the turn: only the open thread.
  expect(change("t1", 2).lists).toEqual([]);
  // It settles with a reply: the lists again.
  expect(change(null, 3).lists).toEqual(threadAndAgents);
  // A hidden group member thread refreshes its group, never the thread list.
  expect(
    nyxAgentLiveRefresh(
      "u",
      { type: "conversation", id: "nyxa-m", group_id: "nyxg-1", turn_id: "t", messages: 1 },
      seen,
    ),
  ).toEqual({
    now: [["assistant", "nyxagent", "u", "groups", "messages", "nyxg-1"]],
    lists: [
      ["assistant", "nyxagent", "u", "groups", "list"],
      ["assistant", "nyxagent", "u", "agents"],
    ],
  });
  // A resync re-reads everything and forgets what it had seen.
  expect(nyxAgentLiveRefresh("u", { type: "resync" }, seen)).toEqual({
    now: [["assistant", "nyxagent", "u"]],
    lists: [],
  });
  expect(change("t1", 2).lists).toEqual(threadAndAgents);
});

it("refreshes titles even when the turn and message count did not change", () => {
  const seen = new Map<string, string>([["nyxa-1", ":2"]]);
  const event = { type: "conversation" as const, id: "nyxa-1", group_id: null, turn_id: null, messages: 2, title_changed: true };
  const refresh = nyxAgentLiveRefresh("u", event, seen);
  expect(refresh.lists).toContainEqual(["assistant", "nyxagent", "u", "threads"]);
  expect(refresh.now).toContainEqual(["assistant", "nyxagent", "u", "history", "nyxa-1"]);
});
