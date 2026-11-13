import assert from "node:assert/strict";
import test from "node:test";
import { fileURLToPath, URL } from "node:url";
import * as timeline from "../../../../shared/onboardingReplay";
import { hookHarness } from "../assistant/assistantHookTestHarness";

type E = { type: unknown; props: Record<string, any> };
function nodes(n: any): E[] {
  return n?.props
    ? [n, ...[n.props.children].flat(Infinity).flatMap(nodes)]
    : [];
}
function setup(
  kind: timeline.LiveReplayKind = "chief-of-staff",
  reduced = false,
) {
  let clock = 0,
    nextTimer = 0;
  const timers = new Map<number, { at: number; fn: () => void }>();
  const listeners = new Set<(state: string) => void>();
  let size = { height: 667, width: 375, fontScale: 1 };
  const h = hookHarness<E>(
    {
      "react/jsx-runtime": {
        jsx: (type: unknown, props: unknown) => ({ type, props }),
        jsxs: (type: unknown, props: unknown) => ({ type, props }),
      },
      "../../../../shared/onboardingReplay": timeline,
      "react-native": {
        View: "View",
        Text: "Text",
        ScrollView: "ScrollView",
        StyleSheet: { create: (s: unknown) => s, hairlineWidth: 0.5 },
        useWindowDimensions: () => size,
        AppState: {
          currentState: "active",
          addEventListener: (_: string, fn: (state: string) => void) => {
            listeners.add(fn);
            return { remove: () => listeners.delete(fn) };
          },
        },
      },
      "../../components/AgentAvatar": { AgentAvatar: "AgentAvatar" },
      "../../components/workspace/WorkspaceUI": { Label: "Label" },
      "../../lib/accessibility/useReducedMotion": {
        useReducedMotion: () => reduced,
      },
      "../../theme/ThemeContext": {
        useTheme: () => ({ colors: {}, mode: "light" }),
      },
      "../../theme/chatAppearance": {
        chatBubbleColors: () => ({ foreground: "bubble-ink" }),
      },
      "../../theme/designTokens": { fonts: {} },
      "../assistant/ChatSurface": {
        ChatMessageTurn: "ChatMessageTurn",
        chatSurfaceStyles: {},
      },
      "../assistant/AssistantReplyBody": {
        AssistantReplyBody: "AssistantReplyBody",
      },
      "../assistant/AssistantAcknowledgementCard": {
        AssistantAcknowledgementCard: "AssistantAcknowledgementCard",
      },
      "../assistant/ChatComposer": {
        ChatComposerFrame: "ChatComposerFrame",
        ChatComposerRow: "ChatComposerRow",
        ComposerSubmitButton: "ComposerSubmitButton",
      },
    },
    {
      setTimeout: (fn: () => void, delay: number) => {
        timers.set(++nextTimer, { at: clock + delay, fn });
        return nextTimer;
      },
      clearTimeout: (id: number) => timers.delete(id),
    },
  );
  h.mock(
    "./useReplayScene",
    h.load(fileURLToPath(new URL("./useReplayScene.ts", import.meta.url))),
  );
  const { OnboardingReplay } = h.load(
    fileURLToPath(new URL("./OnboardingReplay.tsx", import.meta.url)),
  ) as { OnboardingReplay: (props: unknown) => E };
  h.mount(() =>
    OnboardingReplay({ kind, topics: ["calendar"], label: "Example" }),
  );
  const advance = (ms: number) => {
    const until = clock + ms;
    for (;;) {
      const next = [...timers].sort((a, b) => a[1].at - b[1].at)[0];
      if (!next || next[1].at > until) break;
      clock = next[1].at;
      timers.delete(next[0]);
      next[1].fn();
      h.flush();
    }
    clock = until;
  };
  return {
    h,
    timers,
    listeners,
    advance,
    all: () => nodes(h.value()),
    reduce(value: boolean) {
      reduced = value;
      h.render();
    },
    kind(value: timeline.LiveReplayKind) {
      kind = value;
      h.render();
    },
    app(state: string) {
      for (const fn of listeners) fn(state);
      h.flush();
    },
    window(width: number, height: number, fontScale: number) {
      size = { width, height, fontScale };
      h.render();
    },
  };
}
test("native and web share the same topic, specialist and confirmation stages", () => {
  for (const kind of ["chief-of-staff", "specialists", "control"] as const) {
    const f = setup(kind, true);
    const final = timeline.finalScene(
      timeline.replayScript(kind, ["calendar"]),
    );
    for (const entry of final.messages.filter(
      (entry) => entry.role === "assistant",
    ))
      assert.ok(
        f
          .all()
          .some(
            (n) =>
              n.type === "AssistantReplyBody" && n.props.text === entry.content,
          ),
      );
    assert.ok(
      f.all().some((n) => n.type === "ChatMessageTurn" && n.props.user),
    );
    assert.ok(f.all().some((n) => n.type === "ChatComposerFrame"));
    assert.equal(f.timers.size, 0);
    if (kind === "specialists")
      assert.ok(
        f.all().some((n) => n.props.children === "Research replied to NyxBot"),
      );
    if (kind === "control")
      assert.ok(
        f
          .all()
          .some((n) => n.props.children === "Continuing after your decision"),
      );
    f.h.unmount();
  }
});
test("the real replay hook types, presents the real confirmation card, completes and loops", () => {
  const f = setup("control");
  f.advance(700);
  assert.ok(
    f
      .all()
      .some(
        (n) =>
          n.type === "Text" &&
          typeof n.props.children === "string" &&
          n.props.children.startsWith("De"),
      ),
  );
  f.advance(3500);
  assert.ok(
    f
      .all()
      .some(
        (n) =>
          n.type === "AssistantAcknowledgementCard" &&
          n.props.row.status === "pending",
      ),
  );
  f.advance(5000);
  assert.ok(
    f
      .all()
      .some(
        (n) =>
          n.type === "AssistantReplyBody" && n.props.text.startsWith("Done:"),
      ),
  );
  f.advance(2600);
  assert.equal(
    f.all().some((n) => n.type === "ChatMessageTurn"),
    false,
  );
  f.advance(2200);
  assert.ok(f.all().some((n) => n.type === "AssistantAcknowledgementCard"));
  f.h.unmount();
  assert.equal(f.timers.size, 0);
  assert.equal(f.listeners.size, 0);
});
test("Reduce Motion, backgrounding, source changes and unmount stop pending beats", () => {
  const f = setup();
  assert.equal(f.timers.size, 1);
  f.reduce(true);
  assert.equal(f.timers.size, 0);
  assert.ok(f.all().some((n) => n.type === "AssistantReplyBody"));
  f.reduce(false);
  assert.equal(f.timers.size, 1);
  f.app("background");
  assert.equal(f.timers.size, 0);
  f.app("active");
  assert.equal(f.timers.size, 1);
  f.kind("specialists");
  f.advance(700);
  assert.equal(f.timers.size, 1);
  f.h.unmount();
  assert.equal(f.timers.size, 0);
});
test("replay width is bounded on phones, iPad and split windows, with native text and no image or player", () => {
  const f = setup("chief-of-staff", true);
  for (const [w, height, scale] of [
    [375, 667, 1],
    [440, 956, 1],
    [834, 1194, 1],
    [375, 834, 2],
    [640, 375, 2],
  ]) {
    f.window(w!, height!, scale!);
    assert.equal(f.h.value().props.style[0].width, "100%");
    assert.equal(f.h.value().props.style[0].minWidth, 0);
    assert.ok(f.h.value().props.style[1].height <= height! * 0.44);
    assert.equal(
      f.all().some((n) => n.type === "Image" || n.type === "WebView"),
      false,
    );
  }
  f.h.unmount();
});
