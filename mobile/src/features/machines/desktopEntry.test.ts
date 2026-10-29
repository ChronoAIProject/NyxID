import assert from "node:assert/strict";
import test from "node:test";
import {
  desktopEntryParams,
  desktopMetadataSchema,
  desktopRouteParams,
} from "./desktopEntry";
import { createDesktopPrivacyQueue } from "./desktopPrivacy";
import { messageSchema } from "../assistant/assistantTypes";

test("desktop route accepts canonical query selectors and refuses conflicts or invalid display", () => {
  assert.deepEqual(
    desktopRouteParams({
      nodeId: "node",
      conversation_id: "nyxa-thread",
      context_id: "context",
      display: "dev",
    }),
    {
      nodeId: "node",
      conversationId: "nyxa-thread",
      contextId: "context",
      display: "dev",
    },
  );
  assert.throws(
    () =>
      desktopRouteParams({
        nodeId: "node",
        contextId: "one",
        context_id: "two",
      }),
    /Conflicting/,
  );
  assert.throws(() => desktopRouteParams({ nodeId: "../node" }));
  assert.throws(() =>
    desktopRouteParams({ nodeId: "node", display: "unknown" as "secure" }),
  );
});
test("metadata entry preserves the linked conversation, context and developer display", () => {
  const row = desktopMetadataSchema.parse({
    node_id: "node",
    session_id: "session",
    conversation_id: "nyxa-thread",
    context_id: "context",
    display: "dev",
    status: "open",
  });
  assert.deepEqual(desktopEntryParams(row, "wrong-thread"), {
    nodeId: "node",
    conversationId: "nyxa-thread",
    contextId: "context",
    display: "dev",
  });
});
test("native privacy cleanup completes before protection for a new viewer", async () => {
  const queue = createDesktopPrivacyQueue();
  let release!: () => void;
  const gate = new Promise<void>((resolve) => {
    release = resolve;
  });
  const calls: string[] = [];
  const first = queue(async () => {
    calls.push("acquire-old");
    await gate;
  });
  const cleanup = queue(async () => {
    calls.push("release-old");
  });
  const next = queue(async () => {
    calls.push("acquire-new");
  });
  await Promise.resolve();
  assert.deepEqual(calls, ["acquire-old"]);
  release();
  await Promise.all([first, cleanup, next]);
  assert.deepEqual(calls, ["acquire-old", "release-old", "acquire-new"]);
  await assert.rejects(
    queue(async () => {
      throw new Error("missing module");
    }),
  );
  await queue(async () => {
    calls.push("cleanup-after-failure");
  });
  assert.equal(calls.at(-1), "cleanup-after-failure");
});
test("chat history retains machine receipt metadata and drops image/output fields", () => {
  const message = messageSchema.parse({
    id: "m",
    seq: 1,
    turn_id: "t",
    role: "assistant",
    text: "Done",
    status: "completed",
    error_code: null,
    created_at: "now",
    activities: [
      {
        id: "a",
        label: "machine",
        status: "completed",
        started_at: "now",
        machine: {
          node_id: "node",
          action: "computer.snapshot",
          status: "completed",
          context_mode: "separated",
          raw_pixels: "private",
          command_output: "private",
        },
      },
    ],
  });
  assert.equal(message.activities[0]?.machine?.node_id, "node");
  assert.equal("raw_pixels" in message.activities[0]!.machine!, false);
  assert.equal("command_output" in message.activities[0]!.machine!, false);
});
