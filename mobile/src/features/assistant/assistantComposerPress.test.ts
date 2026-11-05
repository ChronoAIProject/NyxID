import assert from "node:assert/strict";
import test from "node:test";
import {
  ASSISTANT_SEND_PRESS_GRACE_MS,
  createAssistantComposerPressGuard,
} from "./assistantComposerPress";

test("an adopted conversation's double Send tap cannot cancel its new turn", () => {
  let time = 1000;
  const accept = createAssistantComposerPressGuard(() => time);
  const requests: string[] = [];
  const tap = (action: "send" | "stop") => {
    if (accept(action)) requests.push(action);
  };

  tap("send");
  time += 100;
  // React has replaced the button before the second native tap arrives.
  tap("stop");
  assert.deepEqual(requests, ["send"]);
});

test("repeat Send callbacks are suppressed and a later deliberate Stop works", () => {
  let time = 1000;
  const accept = createAssistantComposerPressGuard(() => time);
  assert.equal(accept("send"), true);
  time += 100;
  assert.equal(accept("send"), false);
  time = 1000 + ASSISTANT_SEND_PRESS_GRACE_MS - 1;
  assert.equal(accept("stop"), false);
  // The protection expires even when the server has not sent its first delta.
  time++;
  assert.equal(accept("stop"), true);
  time += 1000;
  // Receiving the first delta does not re-arm the Send gesture protection.
  assert.equal(accept("stop"), true);
});

test("a resumed active turn is stoppable without a local Send gesture", () => {
  const accept = createAssistantComposerPressGuard(() => 1000);
  assert.equal(accept("stop"), true);
});
