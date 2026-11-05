import assert from "node:assert/strict";
import test from "node:test";
import { fileURLToPath, URL } from "node:url";
import { deferred, hookHarness } from "../assistant/assistantHookTestHarness";

type E = { type: unknown; props: Record<string, any> };
const nodes = (node: any): E[] =>
  node?.props
    ? [node, ...[node.props.children].flat(Infinity).flatMap(nodes)]
    : [];
function setup() {
  let sheetOpen = false;
  let time = 10000;
  let tick = () => {};
  let epoch = 1,
    focused = true;
  let options: any;
  const events: any[] = [];
  const listeners = new Set<(state: string) => void>();
  const speech: any[] = [];
  const dictation: any = {
    active: false,
    recording: false,
    text: "",
    start: async () => {
      events.push("start");
      dictation.active = true;
    },
    discard: () => {
      events.push("discard");
      dictation.active = false;
    },
  };
  const send = deferred<boolean>();
  const props: any = {
    scope: "crew",
    enabled: true,
    minimized: false,
    draft: "",
    messages: [{ seq: 1, role: "agent", text: "old reply" }],
    onExpand: () => events.push("expand"),
    onClose: () => events.push("close"),
    onSend: (text: string) => {
      events.push(["send", text]);
      return send.promise;
    },
    onChangeText: (text: string) => events.push(["draft", text]),
    canSendText: true,
    onSendText: () => events.push("text-send"),
  };
  const h = hookHarness<E>(
    {
      "react/jsx-runtime": {
        jsx: (type: unknown, props: unknown) => ({ type, props }),
        jsxs: (type: unknown, props: unknown) => ({ type, props }),
      },
      "react-native": {
        ...Object.fromEntries(
          ["Pressable", "ScrollView", "TextInput", "View"].map((n) => [n, n]),
        ),
        StyleSheet: { create: (v: unknown) => v },
        AppState: {
          addEventListener: (_: string, fn: any) => {
            listeners.add(fn);
            return { remove: () => listeners.delete(fn) };
          },
        },
      },
      "@react-navigation/native": { useIsFocused: () => focused },
      "expo-speech": {
        speak: (text: string, options: any) => speech.push({ text, options }),
        stop: async () => {
          events.push("stop-speech");
        },
      },
      "lucide-react-native": {
        Mic: "Mic",
        MicOff: "MicOff",
        PhoneOff: "PhoneOff",
      },
      "../../components/workspace/WorkspaceUI": {
        ActionButton: "Button",
        Label: "Label",
      },
      "../../lib/accessibility/workspaceAccessibility": {
        useWorkspaceAccessibility: () => ({ sheetOpen }),
      },
      "../../lib/auth/authEpoch": { getAuthEpoch: () => epoch },
      "../../theme/ThemeContext": { useTheme: () => ({ colors: {} }) },
      "../../theme/designTokens": { fonts: {} },
      "../assistant/ChatComposer": { ChatComposerFrame: "Frame" },
      "../assistant/useComposerDictation": {
        useComposerDictation: (p: any) => {
          options = p;
          return dictation;
        },
      },
    },
    {
      Date: class extends Date {
        static override now() {
          return time;
        }
      },
      setInterval: (callback: () => void) => {
        tick = callback;
        return 1;
      },
    },
  );
  const Component = h.load(
    fileURLToPath(new URL("./CrewVoiceComposer.tsx", import.meta.url)),
  ).CrewVoiceComposer as any;
  h.mount(() => Component(props));
  return {
    h,
    props,
    events,
    speech,
    dictation,
    send,
    options: () => options,
    sheet: (open: boolean) => {
      sheetOpen = open;
      h.render();
    },
    all: () => nodes(h.value()),
    app: (state: string) => {
      for (const fn of listeners) fn(state);
      h.flush();
    },
    advance: (milliseconds: number) => {
      time += milliseconds;
      tick();
      h.flush();
    },
    background: () => {
      for (const fn of listeners) fn("background");
      h.flush();
    },
    account: () => {
      epoch++;
    },
    blur: () => {
      focused = false;
      h.render();
    },
  };
}

test("crew voice automatically listens, sends native utterances once and retains the session on minimize", async () => {
  const f = setup();
  assert.deepEqual(f.events, ["start"]);
  assert.equal(f.options().continuous, false);
  assert.ok(
    f
      .all()
      .some(
        (n) => n.type === "Label" && String(n.props.children).includes("-"),
      ),
  );
  f.dictation.active = false;
  f.dictation.recording = true;
  f.h.render();
  const delivery = f.options().onInsert("review the launch");
  f.h.flush();
  f.options().onInsert("duplicate");
  assert.equal(
    f.events.filter((e) => Array.isArray(e) && e[0] === "send").length,
    1,
  );
  f.send.resolve(true);
  await delivery;
  f.h.flush();
  f.props.minimized = true;
  f.h.render();
  assert.equal(
    f.all().some((n) => n.type === "ScrollView"),
    false,
  );
  assert.equal(f.events.includes("stop-speech"), false);
  const input = f.all().find((n) => n.type === "TextInput")!;
  input.props.onChangeText("typed");
  input.props.onSubmitEditing();
  assert.deepEqual(f.events.slice(-3), [
    "expand",
    ["draft", "typed"],
    "text-send",
  ]);
  assert.equal(f.events.includes("stop-speech"), true);
  f.h.unmount();
});

test("only new crew replies are read, microphone is released during playback and resumes after the queue", () => {
  const f = setup();
  assert.equal(f.speech.length, 0);
  f.props.messages = [
    ...f.props.messages,
    { seq: 2, role: "user", text: "user message" },
    { seq: 3, role: "agent", text: "first reply" },
  ];
  f.h.render();
  assert.equal(f.speech[0].text, "first reply");
  assert.equal(f.options().enabled, false);
  f.props.messages = [
    ...f.props.messages,
    { seq: 4, role: "agent", text: "second reply" },
  ];
  f.h.render();
  f.speech[0].options.onDone();
  assert.equal(f.speech[1].text, "second reply");
  f.speech[1].options.onDone();
  f.h.flush();
  assert.equal(f.options().enabled, true);
  assert.equal(f.events.filter((e) => e === "start").length, 2);
  f.h.unmount();
});

test("refused sends, speech failures, background, authority changes and unmount stop automatic work", async () => {
  const f = setup();
  const delivery = f.options().onInsert("cannot deliver");
  f.send.resolve(false);
  await delivery;
  f.h.flush();
  assert.ok(f.all().some((n) => n.props.accessibilityRole === "alert"));
  assert.equal(f.options().enabled, false);
  f.background();
  assert.equal(f.options().enabled, false);
  assert.ok(f.events.includes("stop-speech"));
  f.h.unmount();
  const g = setup();
  g.props.messages = [{ seq: 2, role: "agent", text: "live reply" }];
  g.h.render();
  g.speech[0].options.onError();
  g.h.flush();
  assert.ok(g.all().some((n) => n.props.accessibilityRole === "alert"));
  g.account();
  g.speech[0].options.onDone();
  const count = g.speech.length;
  g.h.unmount();
  g.speech[0].options.onDone();
  assert.equal(g.speech.length, count);
  const b = setup();
  b.blur();
  assert.ok(b.events.includes("close"));
  b.h.unmount();
});

test("crew replies arriving during admission are read, and sheets suspend then resume capture", () => {
  const f = setup();
  f.props.busy = true;
  f.h.render();
  f.props.messages = [
    ...f.props.messages,
    { seq: 2, role: "agent", text: "fast reply" },
  ];
  f.h.render();
  assert.equal(f.speech[0].text, "fast reply");
  f.speech[0].options.onDone();
  f.h.flush();
  assert.equal(f.options().enabled, false);
  f.props.busy = false;
  f.h.render();
  assert.equal(f.options().enabled, true);
  f.sheet(true);
  assert.equal(f.options().enabled, false);
  f.sheet(false);
  assert.equal(f.options().enabled, true);
  f.h.unmount();
});

test("typing releases capture and playback before publishing the draft", () => {
  const f = setup();
  const input = f.all().find((node) => node.type === "TextInput")!;
  input.props.onChangeText(" ");
  const discard = f.events.indexOf("discard");
  const stop = f.events.indexOf("stop-speech");
  const edit = f.events.findIndex(
    (event) => Array.isArray(event) && event[0] === "draft",
  );
  assert.ok(discard >= 0 && stop >= 0 && edit > discard && edit > stop);
  f.props.draft = " ";
  f.props.enabled = false;
  f.h.render();
  assert.equal(f.options().enabled, false);
  f.h.unmount();
});

test("permission prompts can return to listening, the clock starts on capture, and background closes crew voice", () => {
  const f = setup();
  const labels = () =>
    f
      .all()
      .filter((n) => n.type === "Label")
      .flatMap((n) => [n.props.children].flat(Infinity));
  f.advance(8000);
  assert.ok(labels().includes("-"));
  f.dictation.phase = "permission";
  f.app("inactive");
  assert.equal(f.events.includes("discard"), false);
  assert.equal(f.events.includes("close"), false);
  f.app("active");
  f.dictation.phase = "listening";
  f.dictation.recording = true;
  f.h.render();
  assert.ok(labels().includes("00:00"));
  f.advance(2000);
  assert.ok(labels().includes("00:02"));
  f.background();
  assert.ok(f.events.includes("discard"));
  assert.ok(f.events.includes("stop-speech"));
  assert.ok(f.events.includes("close"));
  assert.equal(f.options().enabled, false);
  f.h.unmount();
});
