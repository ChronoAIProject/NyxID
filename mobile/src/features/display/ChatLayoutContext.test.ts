import assert from "node:assert/strict";
import test from "node:test";
import { URL, fileURLToPath } from "node:url";
import { deferred, hookHarness } from "../assistant/assistantHookTestHarness";

test("an edit before hydration wins and ordered persistence makes the latest layout durable", async () => {
  const stored = deferred<string | null>();
  const writes: string[] = [];
  const firstWrite = deferred<void>();
  const harness = hookHarness<any>({
    "react/jsx-runtime": {
      jsx: (type: unknown, props: unknown) => ({ type, props }),
    },
    "react-native": {},
    "@react-native-async-storage/async-storage": {
      __esModule: true,
      default: {
        getItem: () => stored.promise,
        setItem: async (_key: string, value: string) => {
          writes.push(value);
          if (writes.length === 1) await firstWrite.promise;
        },
      },
    },
  });
  const source = harness.load(
    fileURLToPath(new URL("./ChatLayoutContext.tsx", import.meta.url)),
  ) as { ChatLayoutProvider: (props: any) => any };
  harness.mount(() => source.ChatLayoutProvider({ children: null }));
  harness.value().props.value.update({ composerMaxHeight: 240 });
  harness.flush();
  stored.resolve('{"composerMaxHeight":80,"columnWidth":360}');
  await harness.settle();
  assert.equal(harness.value().props.value.layout.composerMaxHeight, 240);
  harness.value().props.value.update({ columnWidth: 900 });
  harness.value().props.value.reset();
  await harness.settle();
  assert.equal(writes.length, 1);
  firstWrite.resolve();
  await harness.settle();
  assert.deepEqual(JSON.parse(writes.at(-1)!), {
    columnWidth: 758,
    composerMaxHeight: 120,
  });
  harness.unmount();
});

test("composer height ignores background, oversized and stale third-party events and resyncs after foreground layout", () => {
  type Frame = { screenY: number; height: number };
  const events = new Map<string, (event: { endCoordinates: Frame }) => void>();
  let app: (state: string) => void = () => {};
  let state = "active";
  let metrics: Frame | undefined = { screenY: 552, height: 300 };
  const layouts: Array<() => void> = [];
  const h = hookHarness<number>(
    {
      "react-native": {
        Platform: { OS: "ios" },
        useWindowDimensions: () => ({ height: 852 }),
        AppState: {
          get currentState() {
            return state;
          },
          addEventListener: (_event: string, listener: typeof app) => {
            app = listener;
            return { remove() {} };
          },
        },
        Dimensions: {
          get: () => ({ height: 852 }),
          addEventListener: () => ({ remove() {} }),
        },
        Keyboard: {
          metrics: () => metrics,
          isVisible: () => !!metrics,
          addListener: (
            name: string,
            listener: (event: { endCoordinates: Frame }) => void,
          ) => {
            events.set(name, listener);
            return { remove: () => events.delete(name) };
          },
        },
      },
      "@react-native-async-storage/async-storage": { default: {} },
    },
    {
      requestAnimationFrame: (callback: () => void) => {
        layouts.push(callback);
        return layouts.length;
      },
      cancelAnimationFrame: () => {},
    },
  );
  const source = h.load(
    fileURLToPath(new URL("./ChatLayoutContext.tsx", import.meta.url)),
  ) as { useComposerHeight(): number };
  h.mount(() => source.useComposerHeight());
  assert.equal(h.value(), 120);
  state = "background";
  app(state);
  h.flush();
  events.get("keyboardWillChangeFrame")!({
    endCoordinates: { screenY: 0, height: 1600 },
  });
  h.flush();
  assert.equal(h.value(), 120);
  state = "active";
  app(state);
  h.flush();
  assert.equal(h.value(), 120);
  // The delayed did frame was an earlier accepted will in this show.
  events.get("keyboardWillChangeFrame")!({
    endCoordinates: { screenY: 100, height: 752 },
  });
  events.get("keyboardWillChangeFrame")!({
    endCoordinates: { screenY: 552, height: 300 },
  });
  h.flush();
  events.get("keyboardWillChangeFrame")!({
    endCoordinates: { screenY: 0, height: 1600 },
  });
  events.get("keyboardDidShow")!({
    endCoordinates: { screenY: 100, height: 752 },
  });
  h.flush();
  assert.equal(h.value(), 120);
  metrics = { screenY: 400, height: 452 };
  events.get("keyboardWillChangeFrame")!({ endCoordinates: metrics });
  h.flush();
  assert.equal(h.value(), 96);
  events.get("keyboardDidChangeFrame")!({
    endCoordinates: { screenY: 380, height: 472 },
  });
  h.flush();
  assert.equal(h.value(), 88, "an unseen did-only final frame updates composer limits");
  events.get("keyboardWillChangeFrame")!({
    endCoordinates: { screenY: 400, height: 452 },
  });
  h.flush();
  assert.equal(h.value(), 96);
  events.get("keyboardDidShow")!({
    endCoordinates: { screenY: 100, height: 752 },
  });
  h.flush();
  assert.equal(h.value(), 96);
  metrics = { screenY: 100, height: 752 };
  for (const layout of layouts.splice(0)) layout();
  h.flush();
  assert.equal(h.value(), 96);
  events.get("keyboardDidHide")!({
    endCoordinates: { screenY: 852, height: 0 },
  });
  h.flush();
  assert.equal(h.value(), 120);
  h.unmount();
  assert.equal(events.size, 0);
});

for (const transition of ["inactive overlay", "background rehosting"]) {
  test(`composer height preserves accepted keyboard geometry across ${transition}`, () => {
    type Frame = { screenY: number; height: number };
    const events = new Map<
      string,
      (event: { endCoordinates: Frame }) => void
    >();
    let app: (state: string) => void = () => {};
    let state = "active";
    let metrics: Frame = { screenY: 400, height: 452 };
    const layouts: Array<() => void> = [];
    const h = hookHarness<number>(
      {
        "react-native": {
          Platform: { OS: "ios" },
          useWindowDimensions: () => ({ height: 852 }),
          AppState: {
            get currentState() {
              return state;
            },
            addEventListener: (_event: string, listener: typeof app) => {
              app = listener;
              return { remove() {} };
            },
          },
          Dimensions: {
            get: () => ({ height: 852 }),
            addEventListener: () => ({ remove() {} }),
          },
          Keyboard: {
            metrics: () => metrics,
            isVisible: () => true,
            addListener: (
              name: string,
              listener: (event: { endCoordinates: Frame }) => void,
            ) => {
              events.set(name, listener);
              return { remove: () => events.delete(name) };
            },
          },
        },
        "@react-native-async-storage/async-storage": { default: {} },
      },
      {
        requestAnimationFrame: (callback: () => void) => {
          layouts.push(callback);
          return layouts.length;
        },
        cancelAnimationFrame: () => {},
      },
    );
    const source = h.load(
      fileURLToPath(new URL("./ChatLayoutContext.tsx", import.meta.url)),
    ) as { useComposerHeight(): number };
    h.mount(() => source.useComposerHeight());
    assert.equal(h.value(), 96);
    if (transition === "background rehosting") {
      state = "background";
      app(state);
      h.flush();
      assert.equal(h.value(), 120);
    }
    state = "inactive";
    app(state);
    h.flush();
    assert.equal(h.value(), transition === "inactive overlay" ? 96 : 120);
    events.get("keyboardWillChangeFrame")!({
      endCoordinates: { screenY: -100, height: 1600 },
    });
    h.flush();
    assert.equal(h.value(), transition === "inactive overlay" ? 96 : 120);
    events.get("keyboardWillChangeFrame")!({
      endCoordinates: { screenY: 380, height: 472 },
    });
    h.flush();
    const expected = 88;
    assert.equal(h.value(), expected);
    // The stale metrics cache must not replace the newer inactive frame.
    metrics = { screenY: 100, height: 752 };
    state = "active";
    app(state);
    h.flush();
    assert.equal(h.value(), expected);
    for (const layout of layouts.splice(0)) {
      layout();
      h.flush();
      assert.equal(h.value(), expected);
    }
    h.unmount();
    assert.equal(events.size, 0);
  });
}
