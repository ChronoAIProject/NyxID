import assert from "node:assert/strict";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { hookHarness } from "../../features/assistant/assistantHookTestHarness";
import * as activity from "../../lib/systemStatusActivity";
import { QUICK_RAIL_MAX_STATUS_OFFSET } from "./workspaceLayout";

function setup({ delayed = false, enabled = true, inset = 62.5, platform = "ios" } = {}) {
  let metrics = { width: 466, height: 678, inset };
  let nativeHeight = inset || 62.5;
  const events = new Map<string, (event: any) => void>();
  const reads: ((frame: { height: number }) => void)[] = [];
  let appState: (state: string) => void = () => {};
  const manager = {
    getConstants: () => ({ HEIGHT: 62.5 }),
    getHeight: (callback: (frame: { height: number }) => void) => {
      reads.push(callback);
      if (!delayed) callback({ height: nativeHeight });
    },
  };
  const h = hookHarness<number>({
    "react-native": {
      Platform: { OS: platform },
      NativeModules: { StatusBarManager: manager },
      NativeEventEmitter: class {
        addListener(name: string, callback: (event: any) => void) {
          events.set(name, callback);
          return { remove: () => events.delete(name) };
        }
      },
      AppState: { addEventListener: (_: string, callback: typeof appState) => {
        appState = callback;
        return { remove() {} };
      } },
    },
    "../../lib/systemStatusActivity": activity,
    "./workspaceLayout": { QUICK_RAIL_MAX_STATUS_OFFSET },
  });
  const module = h.load(fileURLToPath(new URL("./useWorkspaceStatusOffset.ts", import.meta.url))) as typeof import("./useWorkspaceStatusOffset");
  h.mount(() => module.useWorkspaceStatusOffset(enabled, metrics.width, metrics.height, metrics.inset));
  return {
    h, events, reads,
    value: () => h.render(),
    frame: (height: number, phase = "Will") => {
      const previous = nativeHeight;
      nativeHeight = height;
      events.get(`statusBarFrame${phase}Change`)?.({ frame: { y: 0, height: phase === "Did" ? previous : height } });
      return h.render();
    },
    window: (width: number, height: number, inset: number) => {
      metrics = { width, height, inset };
      nativeHeight = inset;
      return h.render();
    },
    foreground: () => { appState("active"); return h.render(); },
  };
}

test("idle stays high; a will-change moves before the safe area and returns after the frame settles", () => {
  const x = setup();
  assert.equal(x.value(), 0);
  assert.equal(x.frame(82.5), 20, "react before did-change or a safe-area callback");
  assert.equal(x.frame(110, "Did"), 36, "larger growth stops at the composer alignment");
  assert.equal(x.frame(62.5), 0);
  x.h.unmount();
  assert.equal(x.events.size, 0);
});

test("safe-area growth moves the rail even without status-frame events and resets per posture", () => {
  const x = setup();
  assert.equal(x.window(466, 678, 82.5), 20);
  assert.equal(x.window(466, 678, 120), 36);
  assert.equal(x.window(466, 678, 62.5), 0);
  assert.equal(x.window(951, 669, 10), 0, "unfolding establishes a separate idle baseline");
  assert.equal(x.window(951, 669, 30), 20);
  x.h.unmount();
});

test("initial unmeasured zero insets and hidden status frames do not create false activity", () => {
  const x = setup({ inset: 0, delayed: true });
  assert.equal(x.value(), 0);
  x.reads[0]!({ height: 62.5 });
  assert.equal(x.window(466, 678, 62.5), 0);
  assert.equal(x.frame(0), 0);
  assert.equal(x.frame(62.5), 0);
  x.h.unmount();
});

test("a delayed initial read cannot undo the impending frame; foreground refresh restores a missed idle frame", () => {
  const x = setup({ delayed: true });
  assert.equal(x.frame(98.5), 36);
  x.reads[0]!({ height: 62.5 });
  assert.equal(x.value(), 36);
  x.foreground();
  x.reads.at(-1)!({ height: 62.5 });
  assert.equal(x.value(), 0);
  x.h.unmount();
});

test("camera and microphone owners shift immediately, overlap safely and restore only after both stop", () => {
  const x = setup();
  const camera = activity.claimSystemStatusActivity();
  assert.equal(x.value(), 36, "capture moves before any system geometry event");
  const microphone = activity.claimSystemStatusActivity();
  camera();
  camera();
  assert.equal(x.value(), 36, "idempotent camera release leaves the microphone active");
  microphone();
  assert.equal(x.value(), 0);
  x.h.unmount();
});

test("stopping capture retains displacement while the system icons are still shifted", () => {
  const x = setup();
  const camera = activity.claimSystemStatusActivity();
  x.frame(98.5);
  camera();
  assert.equal(x.value(), 36);
  assert.equal(x.frame(62.5, "Did"), 0);
  x.h.unmount();
});

test("windows without a rail subscribe to no system status events and ignore capture", () => {
  const x = setup({ enabled: false });
  const release = activity.claimSystemStatusActivity();
  assert.equal(x.events.size, 0);
  assert.equal(x.value(), 0);
  release();
  x.h.unmount();
});
