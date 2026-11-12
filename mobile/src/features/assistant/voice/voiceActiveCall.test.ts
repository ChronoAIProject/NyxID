import assert from "node:assert/strict";
import test from "node:test";
import { discoverActiveVoiceCall } from "./voiceActiveCall";

const thread = `nyxa-${"a".repeat(32)}`;

test("an invalid account binding cannot fall back to an unrelated current-chat result", async () => {
  let localReads = 0;
  await assert.rejects(
    discoverActiveVoiceCall(
      {
        accountActive: async () => ({ session: null, conversation_id: thread }),
        active: async () => {
          localReads++;
          return { session: null };
        },
      },
      thread,
    ),
  );
  assert.equal(localReads, 0);
});

test("account discovery failure stays unverified instead of falling back to a misleading empty chat", async () => {
  let localReads = 0;
  const failure = new Error("account state unavailable");
  await assert.rejects(
    discoverActiveVoiceCall(
      {
        accountActive: async () => {
          throw failure;
        },
        active: async () => {
          localReads++;
          return { session: null };
        },
      },
      thread,
    ),
    (error) => error === failure,
  );
  assert.equal(localReads, 0);
});

test("adapters without account discovery preserve the explicit current-chat fallback and abort signal", async () => {
  const signal = new AbortController().signal;
  const reads: unknown[] = [];
  assert.equal(
    await discoverActiveVoiceCall(
      {
        active: async (id, receivedSignal) => {
          reads.push([id, receivedSignal]);
          return { session: null };
        },
      },
      thread,
      signal,
    ),
    null,
  );
  assert.deepEqual(reads, [[thread, signal]]);
});
