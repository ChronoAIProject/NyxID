import assert from "node:assert/strict";
import test from "node:test";
import { observeKeyboardFrames } from "./keyboardFrameEvents";

type Sources = Parameters<typeof observeKeyboardFrames>[0];
type Frame = { screenY: number; height: number };
function setup() {
  const keyboardEvents = new Map<
    string,
    Parameters<Sources["keyboard"]["addListener"]>[1]
  >();
  let appEvent: (state: string) => void = () => {};
  let metrics: Frame | undefined = { screenY: 552, height: 300 };
  const frames: Array<Frame | undefined> = [];
  const layouts = new Map<number, () => void>();
  let sequence = 0;
  let settled = 0;
  const stop = observeKeyboardFrames({
    platform: "ios",
    keyboard: {
      isVisible: () => !!metrics,
      metrics: () => metrics,
      addListener: (name, callback) => {
        keyboardEvents.set(name, callback);
        return {
          remove: () => {
            keyboardEvents.delete(name);
          },
        };
      },
    },
    appState: {
      currentState: "active",
      addEventListener: (_name, callback) => {
        appEvent = callback;
        return { remove() {} };
      },
    },
    dimensions: { addEventListener: () => ({ remove() {} }) },
    windowHeight: () => 852,
    afterLayout: (callback) => {
      layouts.set(++sequence, callback);
      return sequence;
    },
    cancelLayout: (id) => {
      layouts.delete(id);
    },
    onSettled: () => {
      settled++;
    },
    onFrame: (frame) => {
      frames.push(frame);
    },
  });
  return {
    frames,
    settled: () => settled,
    stop,
    app: appEvent,
    metrics: (frame: Frame | undefined) => {
      metrics = frame;
    },
    frame: (frame: Frame, event = "keyboardWillChangeFrame") => {
      keyboardEvents.get(event)!({
        endCoordinates: { ...frame, width: 393, screenX: 0 },
        startCoordinates: { ...frame, width: 393, screenX: 0 },
        duration: 0,
        easing: "keyboard",
      });
    },
    layout: () => {
      for (const [id, callback] of [...layouts]) {
        layouts.delete(id);
        callback();
      }
    },
  };
}

test("inactive system overlays never publish an undefined keyboard frame and keep accepting sanitized changes", () => {
  const env = setup();
  env.app("inactive");
  assert.deepEqual(env.frames, [{ screenY: 552, height: 300 }]);
  env.frame({ screenY: 400, height: 452 });
  env.app("active");
  env.layout();
  assert.ok(env.frames.every((frame) => frame !== undefined));
  assert.deepEqual(env.frames.at(-1), { screenY: 400, height: 452 });
  env.stop();
});

test("background suspends keyboard tracking but inactive rehosting accepts a valid frame and rejects an oversized frame", () => {
  const env = setup();
  env.app("background");
  env.frame({ screenY: 400, height: 452 });
  assert.deepEqual(env.frames, [{ screenY: 552, height: 300 }, undefined]);
  env.app("inactive");
  env.frame({ screenY: -100, height: 1600 });
  assert.equal(env.frames.length, 2);
  env.frame({ screenY: 600, height: 252 });
  env.metrics({ screenY: 100, height: 752 });
  env.app("active");
  env.layout();
  assert.deepEqual(
    env.frames.slice(2),
    Array(3).fill({ screenY: 600, height: 252 }),
  );
  env.stop();
});

test("foreground without a rehosted frame resyncs native visibility and cancels layout work on background", () => {
  const env = setup();
  env.app("background");
  env.metrics({ screenY: 400, height: 452 });
  env.app("inactive");
  env.app("active");
  assert.deepEqual(env.frames.at(-1), { screenY: 400, height: 452 });
  env.app("background");
  const count = env.frames.length;
  env.layout();
  assert.equal(env.frames.length, count);
  assert.equal(env.frames.at(-1), undefined);
  env.stop();
});

for (const event of ["keyboardDidShow", "keyboardDidChangeFrame"]) {
  test(`${event} matches the current show's will history and accepts an unseen final frame`, () => {
    const env = setup();
    const first = { screenY: 636, height: 216 };
    const second = { screenY: 600, height: 252 };
    const final = { screenY: 532, height: 320 };
    env.frame(first);
    const count = env.frames.length;
    env.frame(first, event);
    assert.equal(env.frames.length, count, "latest will duplicate is a no-op");
    env.frame(second);
    env.frame(first, event);
    assert.equal(env.frames.length, count + 1, "superseded will is stale");
    assert.deepEqual(env.frames.at(-1), second);
    env.frame(final, event);
    assert.deepEqual(
      env.frames.at(-1),
      final,
      "did-only final frame is adopted",
    );
    env.frame({ screenY: -100, height: 1600 }, event);
    assert.deepEqual(env.frames.at(-1), final, "did frames are sanitized too");
    env.frame(first, event);
    assert.deepEqual(
      env.frames.at(-1),
      final,
      "late stale did cannot undo the final frame",
    );
    env.frame({ screenY: 852, height: 0 }, "keyboardDidHide");
    env.frame(first, event);
    assert.deepEqual(
      env.frames.at(-1),
      first,
      "a new show starts with an empty history",
    );
    env.stop();
  });
  test(`${event} adopts a third-party final frame when there was only one intermediate will`, () => {
    const env = setup();
    env.frame({ screenY: 636, height: 216 });
    env.frame({ screenY: 532, height: 320 }, event);
    assert.deepEqual(env.frames.at(-1), { screenY: 532, height: 320 });
    env.app("inactive");
    env.app("active");
    env.layout();
    assert.deepEqual(env.frames.at(-1), { screenY: 532, height: 320 });
    env.stop();
  });
}

test("matching and superseded did frames settle native layout without publishing or replacing the accepted frame", () => {
  const env = setup();
  env.frame({ screenY: 636, height: 216 });
  env.frame({ screenY: 532, height: 320 });
  const count = env.frames.length;
  for (const event of ["keyboardDidShow", "keyboardDidChangeFrame"]) {
    env.frame({ screenY: 532, height: 320 }, event);
    env.frame({ screenY: 636, height: 216 }, event);
  }
  assert.equal(env.settled(), 4);
  assert.equal(env.frames.length, count);
  assert.deepEqual(env.frames.at(-1), { screenY: 532, height: 320 });
  env.stop();
});

test("every did event settles before invalid geometry can be rejected", () => {
  const env = setup();
  const frames = env.frames.length;
  for (const event of ["keyboardDidShow", "keyboardDidChangeFrame"]) {
    env.frame({ screenY: -100, height: 1500 }, event);
    env.frame({ screenY: 500, height: Number.NaN }, event);
  }
  assert.equal(env.settled(), 4);
  assert.equal(
    env.frames.length,
    frames,
    "invalid frames never replace accepted geometry",
  );
  env.stop();
});
