import assert from "node:assert/strict";
import test from "node:test";
import { URL, fileURLToPath } from "node:url";
import { deferred, hookHarness } from "../assistant/assistantHookTestHarness";
import * as contracts from "./onboardingContracts";

type E = { type: any; key?: unknown; props: Record<string, any> };
const state = contracts.onboardingSchema.parse({
  show_intro: true,
  intro_version: 1,
  profile: null,
  topics: [
    {
      id: "email",
      label: "Help with all of my email and inboxes",
      icon: "mail",
      work_styles: ["solo"],
    },
  ],
  work_styles: [{ id: "solo", label: "Just me", icon: "user" }],
  recommendations: [
    {
      id: "inbox",
      title: "Connect an inbox",
      description: "Connect your work and personal email safely.",
      prompt: "Connect email",
      icon: "mail",
    },
  ],
});
function nodes(n: any): E[] {
  if (!n?.props) return [];
  if (typeof n.type === "function") return nodes(n.type(n.props));
  return [n, ...[n.props.children].flat(Infinity).flatMap(nodes)];
}
function setup(entryPoint: "first_run" | "home" = "first_run") {
  let epoch = 1;
  let fontScale = 1;
  let nextSave = deferred<typeof state>();
  const events: any[] = [];
  const mutation: any = {
    isPending: false,
    mutateAsync: async (answers: unknown) => {
      events.push(["save", answers]);
      mutation.isPending = true;
      try {
        const result = await nextSave.promise;
        return result;
      } finally {
        mutation.isPending = false;
      }
    },
  };
  let mutationOptions: any;
  const h = hookHarness<E>({
    "react/jsx-runtime": {
      jsx: (type: unknown, props: unknown, key: unknown) => ({
        type,
        props,
        key,
      }),
      jsxs: (type: unknown, props: unknown, key: unknown) => ({
        type,
        props,
        key,
      }),
    },
    "react-native": {
      ...Object.fromEntries(
        ["Image", "Modal", "Pressable", "ScrollView", "View"].map((n) => [
          n,
          n,
        ]),
      ),
      StyleSheet: { create: (v: any) => v },
      Keyboard: { dismiss: () => events.push(["keyboard-dismiss"]) },
      LayoutAnimation: { Types: {}, Properties: {}, configureNext: () => {} },
      useWindowDimensions: () => ({ fontScale }),
    },
    "react-native-safe-area-context": { SafeAreaView: "SafeAreaView" },
    "@expo/vector-icons": { Feather: "Icon" },
    "@tanstack/react-query": {
      useMutation: (options: any) => {
        mutationOptions = options;
        return mutation;
      },
      useQueryClient: () => ({
        setQueryData: (_key: unknown, value: unknown) =>
          events.push(["cache", value]),
      }),
    },
    "../../lib/auth/authEpoch": { getAuthEpoch: () => epoch },
    "../../lib/accessibility/useReducedMotion": {
      useReducedMotion: () => true,
    },
    "../../lib/telemetry": { capture: async () => {} },
    "../../lib/api/onboardingApi": {},
    "../../theme/ThemeContext": { useTheme: () => ({ colors: {} }) },
    "../../theme/designTokens": { fonts: {} },
    "../../components/AgentAvatar": { AgentAvatar: "Avatar" },
    "../../components/workspace/WorkspaceUI": {
      ActionButton: "Button",
      Label: "Label",
    },
    "./onboardingContracts": contracts,
    "../../../assets/onboarding/chief-of-staff.gif": 1,
    "../../../assets/onboarding/specialists.gif": 2,
    "../../../assets/onboarding/control.gif": 3,
  });
  h.mock(
    "./NextStepList",
    h.load(fileURLToPath(new URL("./NextStepList.tsx", import.meta.url))),
  );
  const Intro = h.load(
    fileURLToPath(new URL("./FirstRunIntro.tsx", import.meta.url)),
  ).FirstRunIntro as any;
  const props = {
    visible: true,
    state,
    entryPoint,
    online: true,
    onClose: () => events.push(["close"]),
    onStart: (step: unknown) => events.push(["start", step]),
  };
  h.mount(() => Intro(props));
  return {
    h,
    props,
    events,
    mutation,
    all: () => nodes(h.value()),
    button: (label: string) =>
      nodes(h.value()).find((n) => n.props.label === label)!,
    tab: (index: number) =>
      nodes(h.value()).find(
        (n) => n.props.accessibilityLabel === `Step ${index} of 5`,
      )!,
    resolve: () => nextSave.resolve({ ...state, show_intro: false }),
    reject: () => nextSave.reject(new Error("offline")),
    retry: () => {
      nextSave = deferred<typeof state>();
    },
    resizeType: (next: number) => {
      fontScale = next;
      h.render();
    },
    changeAccount: () => {
      epoch++;
      h.render();
    },
  };
}

test("first-run skip awaits durable success, refuses duplicate taps and exposes retry after failure", async () => {
  const f = setup();
  assert.deepEqual(JSON.parse(JSON.stringify(f.events)), [
    ["keyboard-dismiss"],
  ]);
  f.button("Skip for now").props.onPress();
  f.button("Skip for now").props.onPress();
  assert.equal(f.events.filter((e) => e[0] === "save").length, 1);
  assert.equal(
    f.events.some((e) => e[0] === "close"),
    false,
  );
  f.reject();
  await f.h.settle();
  assert.ok(f.all().some((n) => n.props.accessibilityRole === "alert"));
  assert.equal(
    f.events.some((e) => e[0] === "close"),
    false,
  );
  f.retry();
  f.button("Skip for now").props.onPress();
  f.resolve();
  await f.h.settle();
  assert.deepEqual(
    f.events
      .filter((e) => ["save", "cache", "close"].includes(e[0]))
      .map((e) => e[0]),
    ["save", "save", "cache", "close"],
  );
  assert.equal(f.events.find((e) => e[0] === "save")[1].outcome, "skipped");
  f.h.unmount();
});

test("completion persists selected answers once, then a next step opens only after success", async () => {
  const f = setup("home");
  const pill = f.all().find((n) => n.props.accessibilityRole === "checkbox")!;
  pill.props.onPress();
  f.h.flush();
  f.tab(5).props.onPress();
  f.h.flush();
  assert.deepEqual(
    JSON.parse(JSON.stringify(f.events.find((e) => e[0] === "save")[1])),
    { outcome: "completed", topics: ["email"], work_style: null },
  );
  assert.equal(
    f.events.some((e) => e[0] === "start"),
    false,
  );
  f.resolve();
  await f.h.settle();
  const step = f
    .all()
    .find(
      (n) => n.props.accessibilityRole === "button" && n.type === "Pressable",
    )!;
  await step.props.onPress();
  await f.h.settle();
  assert.deepEqual(
    f.events.slice(-2).map((e) => e[0]),
    ["close", "start"],
  );
  assert.equal(f.events.filter((e) => e[0] === "save").length, 1);
  f.h.unmount();
});

test("offline onboarding is retained and old account or unmounted saves never navigate", async () => {
  for (const leave of ["epoch", "unmount"]) {
    const f = setup();
    f.props.online = false;
    f.h.render();
    assert.equal(f.button("Skip for now").props.disabled, true);
    await f.button("Skip for now").props.onPress();
    assert.equal(
      f.events.some((e) => e[0] === "save"),
      false,
    );
    f.props.online = true;
    f.h.render();
    f.button("Skip for now").props.onPress();
    if (leave === "epoch") f.changeAccount();
    else f.h.unmount();
    f.resolve();
    await f.h.settle();
    assert.equal(
      f.events.some((e) => e[0] === "cache" || e[0] === "close"),
      false,
    );
    if (leave === "epoch") f.h.unmount();
  }
});

test("onboarding actions scroll with long copy, pills wrap, safe areas include both sides and next-step text can shrink", () => {
  const f = setup("home");
  const scroll = f.all().find((n) => n.type === "ScrollView")!;
  assert.equal(scroll.props.style.flex, 1);
  assert.equal(scroll.props.contentContainerStyle.flexGrow, 1);
  assert.equal(scroll.props.contentContainerStyle.width, "100%");
  assert.equal(scroll.props.contentContainerStyle.maxWidth, 680);
  assert.ok(
    nodes(scroll).some((n) => n.props.label === "Continue without picking"),
  );
  assert.deepEqual(
    [...f.all().find((n) => n.type === "SafeAreaView")!.props.edges],
    ["top", "bottom", "left", "right"],
  );
  const pill = f.all().find((n) => n.props.accessibilityRole === "checkbox")!;
  assert.equal(pill.props.style({ pressed: false })[0].maxWidth, "100%");
  assert.equal(pill.props.style({ pressed: false })[0].minHeight, 44);
  assert.equal(f.tab(1).props.style.minHeight, 44);
  assert.equal(f.h.value().props.animationType, "none");
  f.h.unmount();
});

test("live Dynamic Type remeasures onboarding and next steps while preserving answers and progress", () => {
  const f = setup("home");
  f.all()
    .find((n) => n.props.accessibilityRole === "checkbox")!
    .props.onPress();
  f.h.flush();
  const before = f.all().find((n) => n.type === "ScrollView")!.key;
  f.resizeType(3.1);
  assert.notEqual(f.all().find((n) => n.type === "ScrollView")!.key, before);
  assert.equal(
    f.all().find((n) => n.props.accessibilityRole === "checkbox")!.props
      .accessibilityState.checked,
    true,
  );
  assert.equal(f.tab(1).props.accessibilityState.selected, true);
  f.h.unmount();
});
