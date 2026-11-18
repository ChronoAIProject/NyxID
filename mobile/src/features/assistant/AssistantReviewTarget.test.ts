import assert from "node:assert/strict";
import test from "node:test";
import { URL, fileURLToPath } from "node:url";
import { hookHarness } from "./assistantHookTestHarness";
import {
  revealAssistantReviewTarget,
  type ReviewMeasureTarget,
} from "./reviewTargetGeometry";

test("real review target reveals once after layout and fences late account/thread/unmount callbacks", async () => {
  let epoch = 1;
  let props = { selected: true, active: true, scope: "thread-a:card-a" };
  const calls: { target: unknown; heading: unknown; current: () => boolean }[] =
    [];
  const h = hookHarness<any>({
    "react/jsx-runtime": {
      jsx: (type: unknown, props: unknown) => ({ type, props }),
      jsxs: (type: unknown, props: unknown) => ({ type, props }),
    },
    "react-native": { View: "View", Text: "Text" },
    "../../lib/auth/authEpoch": { getAuthEpoch: () => epoch },
    "../../theme/ThemeContext": {
      useTheme: () => ({ colors: { primary: "purple" } }),
    },
  });
  const Target = h.load(
    fileURLToPath(new URL("./AssistantReviewTarget.tsx", import.meta.url)),
  ).AssistantReviewTarget as (props: any) => any;
  h.mount(() =>
    Target({
      ...props,
      children: "Card",
      onReveal: (target: unknown, heading: unknown, current: () => boolean) =>
        calls.push({ target, heading, current }),
    }),
  );
  h.value().props.ref.current = { node: "card" };
  h.value().props.children[0].props.ref.current = { node: "heading" };
  h.value().props.onLayout();
  await new Promise((resolve) => setTimeout(resolve, 10));
  h.flush();
  assert.equal(calls.length, 1);
  assert.equal(calls[0]!.current(), true);
  assert.equal(h.value().props.style.borderColor, "purple");
  h.value().props.onLayout();
  await h.settle();
  assert.equal(
    calls.length,
    1,
    "stream/history rerenders do not repeatedly move the reader",
  );
  epoch++;
  assert.equal(calls[0]!.current(), false);
  props = { ...props, scope: "thread-b:card-a" };
  h.render();
  await new Promise((resolve) => setTimeout(resolve, 10));
  h.flush();
  assert.equal(calls.length, 2);
  props = { ...props, active: false };
  h.render();
  assert.equal(calls[1]!.current(), false);
  props = { ...props, active: true };
  h.render();
  await new Promise((resolve) => setTimeout(resolve, 10));
  h.flush();
  const last = calls.at(-1)!;
  assert.equal(last.current(), true);
  h.unmount();
  assert.equal(last.current(), false);
});

test("review reveal pauses transcript follow, centers the card, focuses its heading and rejects stale measurements", () => {
  const scrolls: number[] = [];
  let focused = 0;
  let paused = 0;
  let allowed = true;
  let viewportMeasurement!: Parameters<
    ReviewMeasureTarget["measureInWindow"]
  >[0];
  let targetMeasurement!: Parameters<ReviewMeasureTarget["measureInWindow"]>[0];
  const reveal = () =>
    revealAssistantReviewTarget({
      viewport: {
        measureInWindow: (cb) => {
          viewportMeasurement = cb;
        },
      },
      target: {
        measureInWindow: (cb) => {
          targetMeasurement = cb;
        },
      },
      current: () => allowed,
      offset: () => 100,
      scroll: (offset) => scrolls.push(offset),
      focus: () => {
        focused++;
      },
      pause: () => {
        paused++;
      },
    });
  reveal();
  assert.equal(paused, 1);
  viewportMeasurement(0, 80, 400, 600);
  targetMeasurement(12, 900, 376, 200);
  assert.deepEqual(scrolls, [720]);
  assert.equal(focused, 1);
  reveal();
  allowed = false;
  viewportMeasurement(0, 80, 400, 600);
  assert.equal(focused, 1);
  allowed = true;
  reveal();
  viewportMeasurement(0, 80, 400, 600);
  allowed = false;
  targetMeasurement(12, 900, 376, 200);
  assert.deepEqual(scrolls, [720]);
  allowed = true;
  reveal();
  viewportMeasurement(0, 80, 400, 600);
  targetMeasurement(12, 0, 376, 800);
  assert.equal(scrolls.at(-1), 4, "tall targets keep a16pt leading inset");
});
