import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { URL, fileURLToPath } from "node:url";
import { runInNewContext } from "node:vm";
import ts from "typescript";
import * as scrollFollower from "./assistantTranscriptScroll";
import { hookHarness } from "./assistantHookTestHarness";
import type { useTranscriptScrollFollow } from "./useTranscriptScrollFollow";

function animationClock() {
  let id = 0;
  const pending = new Map<number, () => void>();
  return {
    globals: {
      requestAnimationFrame: (callback: () => void) => {
        pending.set(++id, callback);
        return id;
      },
      cancelAnimationFrame: (key: number) => pending.delete(key),
    },
    frame: () => {
      const callbacks = [...pending.values()];
      pending.clear();
      callbacks.forEach((callback) => callback());
    },
  };
}

function harness() {
  let scope = "conversation-1";
  let focused = true;
  const follower = scrollFollower.createTranscriptScrollFollower();
  const offsets: number[] = [];
  const list = {
    current: {
      scrollToOffset: ({ offset }: { offset: number; animated: boolean }) => {
        offsets.push(offset);
      },
    },
  };
  const h = hookHarness<ReturnType<typeof useTranscriptScrollFollow>>({
    "./assistantTranscriptScroll": {
      ...scrollFollower,
      createTranscriptScrollFollower: () => follower,
    },
  });
  const hook = h.load(
    fileURLToPath(new URL("./useTranscriptScrollFollow.ts", import.meta.url)),
  ).useTranscriptScrollFollow as typeof useTranscriptScrollFollow;
  h.mount(() => hook(list, scope, { focused }));
  return {
    h,
    offsets,
    follower,
    focus: (next: boolean) => {
      focused = next;
      return h.render();
    },
    switchChat: (next = "conversation-2") => {
      scope = next;
      return h.render();
    },
  };
}

async function restoreFrame(x: ReturnType<typeof harness>) {
  await new Promise<void>((resolve) => setTimeout(resolve, 5));
  x.h.flush();
}

const layout = (height: number) => ({ nativeEvent: { layout: { height } } });
const scroll = (height: number, viewport: number, y: number) => ({
  nativeEvent: {
    contentOffset: { y },
    contentSize: { height },
    layoutMeasurement: { height: viewport },
  },
});

test("native callbacks keep following across a saved reply and keyboard resize", () => {
  const x = harness();
  const props = x.h.value().listProps;
  props.onLayout(layout(400));
  props.onContentSizeChange(300, 1000);
  props.onScroll(scroll(1000, 400, 300));
  props.onContentSizeChange(300, 200);
  props.onScroll(scroll(200, 400, 0));
  props.onContentSizeChange(300, 1100);
  props.onLayout(layout(250));
  props.onScroll(scroll(1100, 250, 700));
  assert.equal(x.offsets.at(-1), 850);
  assert.equal(x.h.flush().following, true);
});

test("user history reading stops corrections until sending resumes following", () => {
  const x = harness();
  let props = x.h.value().listProps;
  props.onLayout(layout(400));
  props.onContentSizeChange(300, 1000);
  props.onScrollBeginDrag();
  props.onScrollEndDrag(scroll(1000, 400, 100));
  assert.equal(x.h.flush().following, false);
  const previous = x.offsets.length;
  props = x.h.value().listProps;
  props.onContentSizeChange(300, 1200);
  props.onLayout(layout(250));
  props.onScroll(scroll(1200, 250, 200));
  assert.equal(x.offsets.length, previous);
  x.h.value().follow();
  assert.equal(x.offsets.at(-1), 950);
  assert.equal(x.h.flush().following, true);
});

test("stale previous-chat callbacks cannot change the new transcript", () => {
  const x = harness();
  const stale = x.h.value();
  stale.listProps.onLayout(layout(400));
  stale.listProps.onContentSizeChange(300, 1000);
  stale.pause();
  x.h.flush();
  const current = x.switchChat();
  const previous = x.offsets.length;
  stale.listProps.onLayout(layout(200));
  stale.listProps.onContentSizeChange(300, 3000);
  stale.listProps.onScrollBeginDrag();
  stale.pause();
  stale.follow();
  assert.equal(x.offsets.length, previous);
  current.listProps.onContentSizeChange(300, 600);
  assert.equal(x.offsets.length, previous);
  current.listProps.onLayout(layout(400));
  assert.equal(x.offsets.at(-1), 200);
  assert.equal(x.h.flush().following, true);
});

test("unmounted native callbacks cannot scroll a replacement list", () => {
  const x = harness();
  const stale = x.h.value();
  stale.listProps.onLayout(layout(400));
  stale.listProps.onContentSizeChange(300, 1000);
  x.h.unmount();
  const previous = x.offsets.length;
  stale.listProps.onContentSizeChange(300, 3000);
  stale.listProps.onLayout(layout(200));
  stale.follow();
  assert.equal(x.offsets.length, previous);
});

test("reopening a chat still rejects its previous list instance's callbacks", () => {
  const x = harness();
  const stale = x.h.value();
  x.switchChat();
  const current = x.switchChat("conversation-1");
  current.listProps.onContentSizeChange(300, 600);
  current.listProps.onLayout(layout(400));
  const previous = x.offsets.length;
  stale.listProps.onLayout(layout(200));
  stale.listProps.onContentSizeChange(300, 3000);
  stale.pause();
  stale.follow();
  assert.equal(x.offsets.length, previous);
  assert.equal(x.h.flush().following, true);
});

test("opening chat details ignores the offscreen native scroll to zero and restores latest", async () => {
  const x = harness();
  const visible = x.h.value();
  visible.listProps.onLayout(layout(811));
  visible.listProps.onContentSizeChange(1000, 6332);
  visible.listProps.onScroll(scroll(6332, 811, 5521));
  const before = x.offsets.length;
  const hidden = x.focus(false);
  for (const props of [visible.listProps, hidden.listProps]) {
    for (const offset of [2622, 102, 0])
      props.onScroll(scroll(6332, 811, offset));
    props.onLayout(layout(0));
    props.onContentSizeChange(1000, 0);
    props.onScrollBeginDrag();
    props.onScrollEndDrag(scroll(6332, 811, 0));
    props.onMomentumScrollBegin();
    props.onMomentumScrollEnd(scroll(6332, 811, 0));
  }
  hidden.pause();
  hidden.follow();
  await restoreFrame(x);
  assert.equal(
    x.offsets.length,
    before,
    "hidden chat must not issue scroll commands",
  );
  x.focus(true);
  await restoreFrame(x);
  assert.equal(x.offsets.at(-1), 5521);
  assert.equal(x.h.value().following, true);
  x.h.unmount();
});

test("returning to a history reader restores the saved offset using current geometry", async () => {
  const x = harness();
  const props = x.h.value().listProps;
  props.onLayout(layout(500));
  props.onContentSizeChange(1000, 4000);
  props.onScrollBeginDrag();
  props.onScrollEndDrag(scroll(4000, 500, 1800));
  x.h.flush();
  assert.equal(x.h.value().following, false);
  x.focus(false).listProps.onScroll(scroll(4000, 500, 0));
  const returning = x.focus(true);
  returning.listProps.onLayout(layout(600));
  returning.listProps.onContentSizeChange(1000, 2000);
  // The return frame can receive a passive native zero before restoration.
  returning.listProps.onScroll(scroll(2000, 600, 0));
  await restoreFrame(x);
  assert.equal(
    x.offsets.at(-1),
    1400,
    "saved offset clamps to the actual shorter chat",
  );
  assert.equal(
    x.h.value().following,
    false,
    "reaching a clamped end does not change reading intent",
  );
  x.h.unmount();
});

test("bottom followers use the new end when return layout and messages change", async () => {
  const x = harness();
  const props = x.h.value().listProps;
  props.onLayout(layout(500));
  props.onContentSizeChange(1000, 3000);
  props.onScroll(scroll(3000, 500, 2500));
  x.focus(false);
  const returning = x.focus(true);
  const before = x.offsets.length;
  returning.listProps.onContentSizeChange(1000, 4000);
  returning.listProps.onLayout(layout(700));
  assert.equal(
    x.offsets.length,
    before,
    "wait for return geometry before one restoration",
  );
  await restoreFrame(x);
  assert.equal(x.offsets.length, before + 1);
  assert.equal(x.offsets.at(-1), 3300);
  assert.equal(x.h.value().following, true);
  x.h.unmount();
});

test("keyboard dismissal while chat details is open updates return geometry without scrolling the hidden chat", async () => {
  const x = harness();
  const visible = x.h.value().listProps;
  visible.onLayout(layout(337));
  visible.onContentSizeChange(1000, 6332);
  visible.onScroll(scroll(6332, 337, 5995));
  const before = x.offsets.length;
  const hidden = x.focus(false).listProps;
  hidden.onLayout(layout(811));
  hidden.onContentSizeChange(1000, 6332);
  hidden.onScroll(scroll(6332, 811, 0));
  // Collapsing native dimensions and previous-cycle callbacks cannot replace
  // the valid keyboard-dismissed geometry or the saved visible position.
  hidden.onLayout(layout(0));
  hidden.onContentSizeChange(1000, 0);
  visible.onLayout(layout(337));
  visible.onContentSizeChange(1000, 9000);
  assert.equal(x.offsets.length, before);
  assert.equal(x.follower.isFollowing(), true);
  x.focus(true);
  // Native keeps the 811-point layout and sends no further layout on Back.
  await restoreFrame(x);
  assert.equal(x.offsets.at(-1), 5521);
  assert.equal(x.follower.distanceToEnd(), 0);
  assert.equal(x.h.value().following, true);
  x.h.unmount();
});

test("hidden positive content and keyboard geometry preserve a history reader's offset and intent", async () => {
  const x = harness();
  const visible = x.h.value().listProps;
  visible.onLayout(layout(337));
  visible.onContentSizeChange(1000, 6332);
  visible.onScrollBeginDrag();
  visible.onScrollEndDrag(scroll(6332, 337, 1400));
  x.h.flush();
  const before = x.offsets.length;
  const hidden = x.focus(false).listProps;
  hidden.onLayout(layout(811));
  hidden.onContentSizeChange(1000, 7000);
  hidden.onScroll(scroll(7000, 811, 0));
  assert.equal(x.offsets.length, before);
  assert.equal(x.follower.isFollowing(), false);
  x.focus(true);
  await restoreFrame(x);
  assert.equal(x.offsets.at(-1), 1400);
  assert.equal(x.follower.distanceToEnd(), 4789);
  assert.equal(x.h.value().following, false);
  x.h.unmount();
});

test("previous focus-cycle callbacks cannot overwrite a restored history position", async () => {
  const x = harness();
  const stale = x.h.value();
  stale.listProps.onLayout(layout(500));
  stale.listProps.onContentSizeChange(1000, 4000);
  stale.listProps.onScrollBeginDrag();
  stale.listProps.onScrollEndDrag(scroll(4000, 500, 1400));
  x.h.flush();
  x.focus(false);
  x.focus(true);
  await restoreFrame(x);
  const restored = x.offsets.length;
  stale.listProps.onLayout(layout(900));
  stale.listProps.onContentSizeChange(1000, 8000);
  stale.listProps.onScroll(scroll(4000, 500, 0));
  stale.follow();
  stale.pause();
  assert.equal(x.offsets.length, restored);
  x.focus(false);
  x.focus(true);
  await restoreFrame(x);
  assert.equal(x.offsets.at(-1), 1400);
  assert.equal(x.h.value().following, false);
  x.h.unmount();
});

test("chat changes, renewed blur and unmount cancel a queued position restoration", async () => {
  for (const boundary of ["chat", "blur", "unmount"] as const) {
    const x = harness();
    const props = x.h.value().listProps;
    props.onLayout(layout(500));
    props.onContentSizeChange(1000, 4000);
    props.onScroll(scroll(4000, 500, 3500));
    x.focus(false);
    x.focus(true);
    const before = x.offsets.length;
    if (boundary === "chat") x.switchChat();
    else if (boundary === "blur") x.focus(false);
    else x.h.unmount();
    await restoreFrame(x);
    assert.equal(x.offsets.length, before, boundary);
    if (boundary !== "unmount") x.h.unmount();
  }
});

test("a deliberate return-frame gesture cancels restoration and keeps the reader in control", async () => {
  const x = harness();
  const props = x.h.value().listProps;
  props.onLayout(layout(500));
  props.onContentSizeChange(1000, 4000);
  props.onScroll(scroll(4000, 500, 3500));
  x.focus(false);
  const returning = x.focus(true);
  const before = x.offsets.length;
  returning.listProps.onScrollBeginDrag();
  returning.listProps.onScrollEndDrag(scroll(4000, 500, 1200));
  await restoreFrame(x);
  assert.equal(x.offsets.length, before);
  assert.equal(x.h.value().following, false);
  x.h.unmount();
});

test("hidden mounting and hidden chat switches do not wait for a nonexistent visible snapshot", async () => {
  for (const boundary of ["mount", "chat"] as const) {
    let focused = boundary === "chat";
    let scope = "chat-1";
    const offsets: number[] = [];
    const follower = scrollFollower.createTranscriptScrollFollower();
    const h = hookHarness<ReturnType<typeof useTranscriptScrollFollow>>({
      "./assistantTranscriptScroll": {
        ...scrollFollower,
        createTranscriptScrollFollower: () => follower,
      },
    });
    const hook = h.load(
      fileURLToPath(new URL("./useTranscriptScrollFollow.ts", import.meta.url)),
    ).useTranscriptScrollFollow as typeof useTranscriptScrollFollow;
    const list = {
      current: {
        scrollToOffset: ({ offset }: { offset: number }) =>
          offsets.push(offset),
      },
    };
    h.mount(() => hook(list, scope, { focused }));
    if (boundary === "chat") {
      h.value().listProps.onLayout(layout(500));
      h.value().listProps.onContentSizeChange(1000, 4000);
      focused = false;
      h.render();
      scope = "chat-2";
      h.render();
    }
    h.value().listProps.onLayout(layout(500));
    h.value().listProps.onContentSizeChange(1000, 4000);
    focused = true;
    h.render();
    await new Promise<void>((resolve) => setTimeout(resolve, 5));
    h.flush();
    // A retained native screen need not repeat its hidden layout on focus.
    // The first visible scroll must still reach the follower immediately.
    h.value().listProps.onContentSizeChange(1000, 4000);
    h.value().listProps.onScroll(scroll(4000, 500, 1400));
    assert.equal(follower.distanceToEnd(), 2100, boundary);
    h.value().listProps.onLayout(layout(500));
    assert.equal(offsets.at(-1), 3500, boundary);
    h.unmount();
  }
});

test("native fling metadata prevents content corrections between drag release and momentum begin", () => {
  for (const nativeRelease of [
    { velocity: { y: -0.5 } },
    { targetContentOffset: { y: 100 } },
  ]) {
    const x = harness();
    const props = x.h.value().listProps;
    props.onLayout(layout(400));
    props.onContentSizeChange(300, 1000);
    props.onScrollBeginDrag();
    props.onScrollEndDrag({
      nativeEvent: { ...scroll(1000, 400, 560).nativeEvent, ...nativeRelease },
    });
    const previous = x.offsets.length;
    props.onContentSizeChange(300, 1200);
    assert.equal(x.offsets.length, previous);
    assert.equal(x.h.flush().following, false);
    props.onMomentumScrollBegin();
    props.onMomentumScrollEnd(scroll(1200, 400, 100));
    assert.equal(x.h.flush().following, false);
  }
});

test("a live footer disappearing during a history drag does not pull the reader to saved rows", () => {
  const x = harness();
  const props = x.h.value().listProps;
  props.onLayout(layout(400));
  props.onContentSizeChange(300, 1200);
  props.onScrollBeginDrag();
  props.onScroll(scroll(1200, 400, 400));
  const previous = x.offsets.length;
  props.onContentSizeChange(300, 260);
  props.onScrollEndDrag(scroll(260, 400, 0));
  props.onContentSizeChange(300, 1000);
  assert.equal(x.offsets.length, previous);
  assert.equal(x.h.flush().following, false);
});

test("jump only appears away from latest, and resumes follow after explicit activation", () => {
  const x = harness();
  let p = x.h.value().listProps;
  p.onLayout(layout(400));
  p.onContentSizeChange(300, 1200);
  p.onScroll(scroll(1200, 400, 800));
  x.h.value().pause();
  assert.equal(x.h.flush().showJump, false);
  p.onScrollBeginDrag();
  p.onScrollEndDrag(scroll(1200, 400, 100));
  p.onScroll(scroll(1200, 400, 100));
  assert.equal(x.h.flush().showJump, true);
  x.h.value().follow();
  assert.equal(x.h.flush().showJump, false);
  assert.equal(x.offsets.at(-1), 800);
  x.h.unmount();
});

test("corrections glide after opening: jump and small growth animate, large jumps and Reduce Motion stay instant", () => {
  for (const reducedMotion of [false, true]) {
    const calls: { offset: number; animated: boolean }[] = [];
    const list = {
      current: {
        scrollToOffset: (call: { offset: number; animated: boolean }) =>
          calls.push(call),
      },
    };
    const clock = animationClock();
    const h = hookHarness<ReturnType<typeof useTranscriptScrollFollow>>(
      {
        "./assistantTranscriptScroll": scrollFollower,
      },
      clock.globals,
    );
    const hook = h.load(
      fileURLToPath(new URL("./useTranscriptScrollFollow.ts", import.meta.url)),
    ).useTranscriptScrollFollow as typeof useTranscriptScrollFollow;
    h.mount(() => hook(list, "chat", { reducedMotion }));
    const p = h.value().listProps;
    p.onLayout(layout(400));
    p.onContentSizeChange(300, 4000);
    assert.equal(
      calls.at(-1)!.animated,
      false,
      "opening a long chat jumps straight to the end",
    );
    p.onScroll(scroll(4000, 400, 3600));
    clock.frame();
    clock.frame();
    h.flush();
    p.onContentSizeChange(300, 4200);
    assert.equal(
      calls.at(-1)!.animated,
      !reducedMotion,
      "a growing reply is followed smoothly",
    );
    p.onScroll(scroll(4200, 400, 3800));
    p.onContentSizeChange(300, 6000);
    assert.equal(
      calls.at(-1)!.animated,
      false,
      "a jump of several screens is instant",
    );
    p.onScrollBeginDrag();
    p.onScrollEndDrag(scroll(6000, 400, 100));
    p.onScroll(scroll(6000, 400, 100));
    h.value().follow();
    assert.equal(
      calls.at(-1)!.animated,
      !reducedMotion,
      "jump to latest glides",
    );
    h.unmount();
  }
});

test("opening stays hidden through loading and geometry changes until native latest is acknowledged", () => {
  let loading = true;
  const clock = animationClock();
  const h = hookHarness<ReturnType<typeof useTranscriptScrollFollow>>(
    { "./assistantTranscriptScroll": scrollFollower },
    clock.globals,
  );
  const hook = h.load(
    fileURLToPath(new URL("./useTranscriptScrollFollow.ts", import.meta.url)),
  ).useTranscriptScrollFollow as typeof useTranscriptScrollFollow;
  const calls: { offset: number; animated: boolean }[] = [];
  h.mount(() =>
    hook({ current: { scrollToOffset: (c) => calls.push(c) } }, "chat", {
      loading,
    }),
  );
  let p = h.value().listProps;
  p.onLayout(layout(400));
  p.onContentSizeChange(300, 1000);
  p.onScroll(scroll(1000, 400, 600));
  clock.frame();
  clock.frame();
  assert.equal(h.flush().ready, false);
  loading = false;
  h.render();
  p = h.value().listProps;
  p.onContentSizeChange(300, 1600);
  p.onLayout(layout(300));
  clock.frame();
  clock.frame();
  assert.equal(
    h.flush().ready,
    false,
    "the requested offset alone cannot reveal the list",
  );
  assert.equal(calls.at(-1)!.offset, 1300);
  p.onScroll(scroll(1600, 300, 1300));
  clock.frame();
  clock.frame();
  assert.equal(h.flush().ready, true);
  assert.equal(
    calls.some((c) => c.animated),
    false,
  );
  h.unmount();
});

test("opening reveals by 800ms despite loading, no native acknowledgement or changing geometry", () => {
  for (const condition of ["loading", "no-ack", "changing-geometry"]) {
    let now = 0,
      id = 0;
    const timers = new Map<number, { at: number; callback: () => void }>();
    const clock = animationClock();
    let scope = "first";
    let historyError = false;
    const h = hookHarness<ReturnType<typeof useTranscriptScrollFollow>>(
      { "./assistantTranscriptScroll": scrollFollower },
      {
        ...clock.globals,
        setTimeout: (callback: () => void, delay: number) => {
          timers.set(++id, { at: now + delay, callback });
          return id;
        },
        clearTimeout: (key: number) => timers.delete(key),
      },
    );
    const advance = (ms: number) => {
      now += ms;
      for (const [key, timer] of [...timers])
        if (timer.at <= now) {
          timers.delete(key);
          timer.callback();
        }
      h.flush();
    };
    const hook = h.load(
      fileURLToPath(new URL("./useTranscriptScrollFollow.ts", import.meta.url)),
    ).useTranscriptScrollFollow as typeof useTranscriptScrollFollow;
    h.mount(() =>
      hook({ current: { scrollToOffset: () => {} } }, scope, {
        loading: condition === "loading",
        historyError,
      }),
    );
    h.value().listProps.onLayout(layout(400));
    h.value().listProps.onContentSizeChange(300, 2000);
    advance(799);
    if (condition === "changing-geometry") {
      for (let i = 0; i < 10; i++) {
        h.value().listProps.onContentSizeChange(300, 2000 + i * 10);
        clock.frame();
      }
    }
    assert.equal(h.value().ready, false, condition);
    advance(1);
    assert.equal(h.value().ready, true, condition);
    assert.equal(timers.size, 0);
    scope = "second";
    h.render();
    assert.equal(h.value().ready, false);
    assert.equal(timers.size, 1);
    advance(400);
    scope = "third";
    h.render();
    advance(400);
    assert.equal(
      h.value().ready,
      false,
      "the prior scope's deadline is cancelled",
    );
    historyError = true;
    h.render();
    assert.equal(h.value().ready, true, "history errors reveal immediately");
    assert.equal(timers.size, 0);
    scope = "fourth";
    historyError = false;
    h.render();
    h.unmount();
    assert.equal(timers.size, 0, "unmount clears deadline");
  }
});

test("an abandoned conversation render cannot strand the committed chat behind its opening overlay", () => {
  type Effect = {
    slot: number;
    dependencies: unknown[];
    run: () => void | (() => void);
    cleanup?: () => void;
  };
  const slots: unknown[] = [];
  const effects = new Map<number, Effect>();
  const timers = new Map<number, { at: number; callback: () => void }>();
  let pending: Effect[] = [];
  let cursor = 0;
  let now = 0;
  let timerId = 0;
  const react = {
    useRef: <T>(initial: T) => {
      const slot = cursor++;
      return (slots[slot] ??= { current: initial });
    },
    useState: <T>(initial?: T) => {
      const slot = cursor++;
      if (!(slot in slots)) slots[slot] = initial;
      return [slots[slot], (next: T) => (slots[slot] = next)];
    },
    useEffect: (run: Effect["run"], dependencies: unknown[]) => {
      const slot = cursor++;
      const committed = effects.get(slot);
      if (
        !committed ||
        dependencies.length !== committed.dependencies.length ||
        dependencies.some(
          (value, index) => !Object.is(value, committed.dependencies[index]),
        )
      )
        pending.push({ slot, dependencies, run });
    },
    useLayoutEffect: (run: Effect["run"], dependencies: unknown[]) => {
      react.useEffect(run, dependencies);
    },
  };
  const filename = fileURLToPath(
    new URL("./useTranscriptScrollFollow.ts", import.meta.url),
  );
  const exports: Record<string, unknown> = {};
  runInNewContext(
    ts.transpileModule(readFileSync(filename, "utf8"), {
      compilerOptions: {
        module: ts.ModuleKind.CommonJS,
        target: ts.ScriptTarget.ES2022,
      },
    }).outputText,
    {
      exports,
      require: (name: string) => {
        if (name === "react") return react;
        if (name === "./assistantTranscriptScroll") return scrollFollower;
        throw new Error(`Unexpected hook dependency: ${name}`);
      },
      setTimeout: (callback: () => void, delay: number) => {
        timers.set(++timerId, { at: now + delay, callback });
        return timerId;
      },
      clearTimeout: (id: number) => timers.delete(id),
      ...animationClock().globals,
    },
    { filename },
  );
  const hook =
    exports.useTranscriptScrollFollow as typeof useTranscriptScrollFollow;
  const render = (scope: string, commit = true) => {
    cursor = 0;
    pending = [];
    const result = hook({ current: null }, scope, { loading: true });
    // Shared ref writes survive abandoned renders, but only committed effects
    // replace the last dependencies and run their cleanup/setup callbacks.
    if (commit) {
      for (const effect of pending) effects.get(effect.slot)?.cleanup?.();
      for (const effect of pending) {
        effect.cleanup = effect.run() || undefined;
        effects.set(effect.slot, effect);
      }
    }
    return result;
  };
  const advance = (ms: number) => {
    now += ms;
    for (const [id, timer] of [...timers]) {
      if (timer.at > now) continue;
      timers.delete(id);
      timer.callback();
    }
    return render("first");
  };

  assert.equal(render("first").ready, false);
  const firstDeadline = [...timers.values()][0];
  assert.ok(
    firstDeadline,
    "the committed render installs a readiness deadline",
  );
  const obsoleteDeadline = firstDeadline.callback;
  const firstTimer = timerId;
  render("abandoned", false);
  assert.equal(
    timerId,
    firstTimer,
    "discarding a render cannot install effects",
  );
  assert.equal(render("first").ready, false);
  assert.equal(
    timers.size,
    1,
    "the committed generation replaces its deadline",
  );
  obsoleteDeadline();
  assert.equal(
    render("first").ready,
    false,
    "obsolete deadlines cannot reveal",
  );
  const committedTimer = timerId;
  assert.equal(advance(400).ready, false);
  assert.equal(
    timerId,
    committedTimer,
    "same-chat renders preserve the deadline",
  );
  assert.equal(advance(399).ready, false);
  assert.equal(
    advance(1).ready,
    true,
    "the current chat is reachable by 800ms",
  );
  assert.equal(timers.size, 0);
  render("next");
  assert.equal(timers.size, 1);
  for (const effect of effects.values()) effect.cleanup?.();
  assert.equal(timers.size, 0, "unmount clears the replacement deadline");
});
