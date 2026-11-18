import { WORKSPACE_HEADER_ROW_HEIGHT } from "../../components/workspace/workspaceHeaderMetrics";
import assert from "node:assert/strict";
import test from "node:test";
import { URL, fileURLToPath } from "node:url";
import { hookHarness } from "./assistantHookTestHarness";

type E = { type: unknown; props: Record<string, any> };
const nodes = (n: any): E[] =>
  n?.props ? [n, ...[n.props.children].flat(Infinity).flatMap(nodes)] : [];
const text = (n: any): string =>
  typeof n === "string" || typeof n === "number"
    ? String(n)
    : n?.props
      ? [n.props.children].flat(Infinity).map(text).join(" ")
      : "";
const colors = {
  bg: "background",
  textPrimary: "primary",
  textSecondary: "secondary",
  textMuted: "muted",
  warningTone: { text: "warning" },
};
const mocks = {
  "react/jsx-runtime": {
    jsx: (type: unknown, props: unknown) => ({ type, props }),
    jsxs: (type: unknown, props: unknown) => ({ type, props }),
  },
  "react-native": {
    View: "View",
    Text: "Text",
    Pressable: "Pressable",
    StyleSheet: { create: (value: unknown) => value },
  },
  "react-native-safe-area-context": { useSafeAreaInsets: () => ({ top: 62 }) },
  "lucide-react-native": { ChevronDown: "ChevronDown" },
  "../../components/AgentAvatar": {
    AgentAvatar: "AgentAvatar",
    AgentAvatarStack: "AgentAvatarStack",
  },
  "../../theme/ThemeContext": { useTheme: () => ({ colors }) },
};
function setup() {
  const h = hookHarness<E>(mocks);
  const module = h.load(
    fileURLToPath(new URL("./ChatIdentityHeader.tsx", import.meta.url)),
  ) as {
    ChatIdentity: (props: any) => E;
    ChatHeaderFrame: (props: any) => E;
    chatThreadInfo: (...args: any[]) => string;
  };
  return { h, ...module };
}
const bot = {
  id: "bot",
  name: "nyxbot",
  display_name: "NyxBot",
  kind: "nyxbot",
};
const specialist = {
  id: "research",
  name: "researcher",
  display_name: "Research team agent",
  kind: "specialist",
};

test("thread header info uses the title and compact activity, handle fallback, or New chat", () => {
  const x = setup();
  const now = Date.parse("2026-10-28T08:00:00Z");
  assert.equal(
    x.chatThreadInfo("Release notes", "bot", "2026-10-28T06:00:00Z", now),
    "Release notes · 2h",
  );
  assert.equal(x.chatThreadInfo(null, " @researcher "), "@researcher");
  assert.equal(x.chatThreadInfo("  ", ""), "New chat");
  assert.equal(x.chatThreadInfo("Draft", "bot", "invalid"), "Draft");
});

test("NyxBot and specialists have xl avatars beside two truncating, scaling name/thread lines", () => {
  for (const agent of [bot, specialist]) {
    const x = setup();
    let opened = 0;
    x.h.mount(() =>
      x.ChatIdentity({
        agent,
        name: agent.display_name,
        detail: "Release notes · 2h",
        status: "Needs you",
        onPress: () => opened++,
      }),
    );
    const all = nodes(x.h.value());
    const target = all.find((n) => n.type === "Pressable")!;
    assert.equal(
      target.props.accessibilityLabel,
      `Chatting with ${agent.display_name}, Release notes · 2h, Needs you. Choose agent`,
    );
    assert.equal(target.props.style.minHeight, 44);
    target.props.onPress();
    assert.equal(opened, 1);
    const avatar = all.find((n) => n.type === "AgentAvatar")!;
    assert.equal(avatar.props.agent.id, agent.id);
    assert.equal(avatar.props.size, "xl");
    const lines = all.filter(
      (n) => n.type === "Text" && n.props.ellipsizeMode === "tail",
    );
    assert.equal(lines.length, 2);
    for (const line of lines) {
      assert.equal(line.props.numberOfLines, 1);
      assert.equal(line.props.allowFontScaling, true);
      assert.ok(
        line.props.maxFontSizeMultiplier > 1 &&
          line.props.maxFontSizeMultiplier <= 1.4,
      );
    }
    assert.equal(lines[0]!.props.style[0].fontSize, 16);
    assert.equal(lines[1]!.props.style[0].fontSize, 11);
    assert.equal(lines[1]!.props.style[1].color, colors.textMuted);
    assert.ok(all.some((n) => n.type === "ChevronDown"));
    x.h.unmount();
  }
});

test("crew identity uses the shared avatar stack, full crew/member labels and disabled picker guard", () => {
  const x = setup();
  x.h.mount(() =>
    x.ChatIdentity({
      name: "Launch crew",
      detail: "NyxBot, Research team agent",
      members: [bot, specialist],
      status: "Running",
      disabled: true,
      onPress: () => {},
    }),
  );
  const all = nodes(x.h.value());
  const avatar = all.find((n) => n.type === "AgentAvatarStack")!;
  assert.equal(avatar.props.size, "xl");
  assert.equal(avatar.props.agents.length, 2);
  assert.equal(avatar.props.max, 2);
  assert.equal(x.h.value().props.disabled, true);
  assert.equal(x.h.value().props.accessibilityState.disabled, true);
  assert.match(
    x.h.value().props.accessibilityLabel,
    /Chatting with Launch crew, NyxBot, Research team agent, Running\. Choose agent/,
  );
  assert.match(text(x.h.value()), /Launch crew.*NyxBot, Research team agent/);
  x.h.unmount();
});

test("shared custom header anchors Menu/Back then flexible identity then existing right controls", () => {
  for (const back of [undefined, { title: "Back" }]) {
    const x = setup();
    const calls: any[] = [];
    x.h.mount(() => {
      const element = x.ChatHeaderFrame({
        back,
        route: { name: "Assistant" },
        options: {
          headerLeft: (props: any) => {
            calls.push(props);
            return { type: "MenuOrBack", props: {} };
          },
          headerTitle: () => ({ type: "Identity", props: {} }),
          headerRight: () => ({ type: "NewChatAndOptions", props: {} }),
        },
      });
      return (element.type as (props: any) => E)(element.props);
    });
    assert.equal(x.h.value().type, "View");
    assert.equal(x.h.value().props.style.paddingTop, 62);
    const row = x.h.value().props.children;
    assert.equal(row.props.style.flexDirection, "row");
    assert.equal(row.props.style.height, WORKSPACE_HEADER_ROW_HEIGHT);
    const children = row.props.children;
    assert.equal(children[0].type, "MenuOrBack");
    assert.equal(children[1].props.style.flex, 1);
    assert.equal(children[1].props.style.minWidth, 0);
    assert.equal(children[1].props.children.type, "Identity");
    assert.equal(children[2].type, "NewChatAndOptions");
    assert.equal(calls[0].canGoBack, !!back);
    x.h.unmount();
  }
});

test("navigation can invoke chat header options repeatedly without calling hooks", () => {
  let hookCalls = 0;
  const h = hookHarness<E>({
    ...mocks,
    "../../theme/ThemeContext": {
      useTheme: () => {
        hookCalls++;
        return { colors };
      },
    },
  });
  const { ChatHeaderFrame } = h.load(
    fileURLToPath(new URL("./ChatIdentityHeader.tsx", import.meta.url)),
  ) as { ChatHeaderFrame: (props: any) => E };
  for (const state of ["home", "private", "crew", "home"]) {
    const options = {
      header: ChatHeaderFrame,
      title: state,
      headerTitle: () => ({ type: "Identity", props: { name: state } }),
    };
    const element = options.header({
      options,
      route: { name: state },
      back: state === "crew" ? {} : undefined,
    });
    assert.equal(hookCalls, 0, "option callback must not run component hooks");
    assert.equal(typeof element.type, "function");
    h.mount(() => (element.type as (props: any) => E)(element.props));
    assert.equal(hookCalls, 1);
    h.unmount();
    hookCalls = 0;
  }
});
