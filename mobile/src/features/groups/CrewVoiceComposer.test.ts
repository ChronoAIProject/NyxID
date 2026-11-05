import assert from "node:assert/strict";
import test from "node:test";
import { fileURLToPath, URL } from "node:url";
import { deferred, hookHarness } from "../assistant/assistantHookTestHarness";

type E = { type: unknown; props: Record<string, any> };
function nodes(node: any): E[] {
  if (!node?.props) return [];
  if (typeof node.type === "function") return nodes(node.type(node.props));
  return [node, ...[node.props.children].flat(Infinity).flatMap(nodes)];
}
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
    subscribeLevel: () => () => {},
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
    crewName: "Launch crew",
    recipientLabel: "Researcher and Designer",
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
          ["Pressable", "ScrollView", "TextInput", "Text", "View"].map((n) => [
            n,
            n,
          ]),
        ),
        StyleSheet: { create: (v: unknown) => v },
        useWindowDimensions: () => ({ height: 844 }),
        Keyboard: { addListener: () => ({ remove: () => {} }) },
        AppState: {
          addEventListener: (_: string, fn: any) => {
            listeners.add(fn);
            return { remove: () => listeners.delete(fn) };
          },
        },
      },
      "react-native-safe-area-context": {
        useSafeAreaInsets: () => ({ top: 47, bottom: 34 }),
      },
      "react-native-reanimated": { useSharedValue: () => ({ value: 0 }) },
      "../../lib/accessibility/useReducedMotion": {
        useReducedMotion: () => true,
      },
      "./VoiceComposerAura": { VoiceComposerAura: "VoiceAura" },
      "../ChatComposer": { ChatComposerFrame: "Frame" },
      "../../../theme/ThemeContext": { useTheme: () => ({ colors: {} }) },
      "../../../theme/designTokens": { fonts: {} },
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
  h.mock(
    "../assistant/voice/VoiceComposerShell",
    h.load(
      fileURLToPath(
        new URL("../assistant/voice/VoiceComposerShell.tsx", import.meta.url),
      ),
    ),
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
  assert.ok(f.all().some((n) => n.props.accessibilityLabel === "Call time -"));
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
  assert.deepEqual(f.events.slice(-2), [["draft", "typed"], "text-send"]);
  assert.ok(f.events.includes("expand"));
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
  f.advance(8000);
  assert.ok(f.all().some((n) => n.props.accessibilityLabel === "Call time -"));
  f.dictation.phase = "permission";
  f.app("inactive");
  assert.equal(f.events.includes("discard"), false);
  assert.equal(f.events.includes("close"), false);
  f.app("active");
  f.dictation.phase = "listening";
  f.dictation.recording = true;
  f.h.render();
  assert.ok(
    f.all().some((n) => n.props.accessibilityLabel === "Call time 00:00"),
  );
  f.advance(2000);
  assert.ok(
    f.all().some((n) => n.props.accessibilityLabel === "Call time 00:02"),
  );
  f.background();
  assert.ok(f.events.includes("discard"));
  assert.ok(f.events.includes("stop-speech"));
  assert.ok(f.events.includes("close"));
  assert.equal(f.options().enabled, false);
  f.h.unmount();
});

test("crew shell shares aura, header, central mic, End and compact row with private voice", () => {
  const f = setup();
  assert.ok(f.all().some((n) => n.type === "VoiceAura"));
  assert.ok(
    f
      .all()
      .some((n) => n.type === "Text" && n.props.children === "Launch crew"),
  );
  const mic = f
    .all()
    .find((n) => n.props.accessibilityLabel === "Pause crew microphone")!;
  assert.equal(mic.props.style({ pressed: false })[0].width, 64);
  assert.ok(
    f.all().some((n) => n.props.accessibilityLabel === "End voice and close"),
  );
  f.props.minimized = true;
  f.h.render();
  assert.equal(
    f.all().some((n) => n.type === "ScrollView"),
    false,
  );
  assert.ok(
    f.all().some((n) => n.props.accessibilityLabel === "Expand voice composer"),
  );
  assert.ok(f.all().some((n) => n.props.accessibilityLabel === "Call time -"));
  f.h.unmount();
});
test("temporary inactivity re-arms capture but explicit user pause survives app activation", () => {
  const f = setup();
  f.dictation.phase = "listening";
  f.app("inactive");
  assert.equal(f.options().enabled, false);
  f.app("active");
  assert.equal(f.options().enabled, true);
  assert.equal(f.events.filter((e) => e === "start").length, 2);
  f.all()
    .find((n) => n.props.accessibilityLabel === "Pause crew microphone")!
    .props.onPress();
  f.h.flush();
  assert.equal(f.options().enabled, false);
  f.app("inactive");
  f.app("active");
  assert.equal(f.options().enabled, false);
  f.all()
    .find((n) => n.props.accessibilityLabel === "Resume crew microphone")!
    .props.onPress();
  f.h.flush();
  assert.equal(f.options().enabled, true);
  f.background();
  f.app("active");
  assert.equal(
    f.options().enabled,
    false,
    "a closed background session never resumes",
  );
  f.h.unmount();
});
test("silent native utterances re-arm capture while recognizer errors require explicit retry", () => {
  const f = setup();
  f.h.render();
  f.dictation.active = false;
  f.h.render();
  assert.equal(f.events.filter((e) => e === "start").length, 2);
  f.dictation.active = false;
  f.dictation.error = "Microphone denied";
  f.h.render();
  assert.equal(f.events.filter((e) => e === "start").length, 2);
  assert.ok(
    f.all().some((n) => n.props.accessibilityLabel === "Retry crew voice"),
  );
  f.h.unmount();
});
test("a reply arriving while the spoken send promise is pending is read as plain text", async () => {
  const f = setup();
  const delivery = f.options().onInsert("send this");
  f.h.flush();
  f.props.messages = [
    ...f.props.messages,
    {
      seq: 2,
      role: "agent",
      text: "**Done.** Read [the plan](https://example.com).",
    },
  ];
  f.h.render();
  assert.equal(f.speech[0].text, "Done. Read the plan.");
  f.send.resolve(true);
  await delivery;
  f.h.flush();
  f.h.unmount();
});

test("a stopped playback's late error cannot stop a newer reply", () => {
  const f = setup();
  f.props.messages = [
    ...f.props.messages,
    { seq: 2, role: "agent", text: "first" },
  ];
  f.h.render();
  const stale = f.speech[0].options;
  f.app("inactive");
  f.app("active");
  f.props.messages = [
    ...f.props.messages,
    { seq: 3, role: "agent", text: "second" },
  ];
  f.h.render();
  const stops = f.events.filter((e) => e === "stop-speech").length;
  stale.onError();
  f.h.flush();
  assert.equal(f.events.filter((e) => e === "stop-speech").length, stops);
  assert.equal(
    f.all().some((n) => n.props.accessibilityRole === "alert"),
    false,
  );
  assert.equal(f.speech[1].text, "second");
  f.h.unmount();
});
