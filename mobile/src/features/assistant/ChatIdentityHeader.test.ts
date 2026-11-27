import { WORKSPACE_HEADER_ROW_HEIGHT } from "../../components/workspace/workspaceHeaderMetrics";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { URL, fileURLToPath } from "node:url";
import { hookHarness } from "./assistantHookTestHarness";

/** Static row styles may be split across an array (the slab-motion slot). */
const flatStyle = (node: { props: Record<string, any> }) =>
  Object.assign({}, ...[node.props.style].flat(Infinity).filter(Boolean));

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
  composerBg: "composer",
  composerBorder: "composer-border",
  shadowColor: "shadow",
};
let window = { width: 402, height: 874 };
const mocks = {
  "react/jsx-runtime": {
    jsx: (type: unknown, props: unknown) => ({ type, props }),
    jsxs: (type: unknown, props: unknown) => ({ type, props }),
  },
  "react-native": {
      Platform: {OS: "ios"},
    View: "View",
    Text: "Text",
    Pressable: "Pressable",
    StyleSheet: { create: (value: unknown) => value, hairlineWidth: 0.5 },
    useWindowDimensions: () => window,
  },
  "react-native-safe-area-context": { useSafeAreaInsets: () => ({ top: 62 }) },
  "lucide-react-native": { ChevronDown: "ChevronDown" },
  "../../components/AgentAvatar": {
    AgentAvatar: "AgentAvatar",
    AgentAvatarStack: "AgentAvatarStack",
  },
  "../../theme/ThemeContext": { useTheme: () => ({ colors, mode: "light" }) },
  "expo-linear-gradient": { LinearGradient: "LinearGradient" },
  "../../components/workspace/WorkspaceSlabMotion": {
    MotionView: "MotionView",
    HEADER_ACTION_ENTERING: "fade-in",
    HEADER_ACTION_EXITING: "fade-out",
  },
};
const flat = (style: any) =>
  Object.assign(
    {},
    ...[typeof style === "function" ? style({ pressed: false }) : style]
      .flat(Infinity)
      .filter(Boolean),
  );
/** The bar itself, inside its collapsed-bar context and collapsing frame. */
const bar = (root: E) =>
  nodes(root).find((n) => n.props.testID === "workspace-chat-header")!;
const headerRow = (root: E) =>
  nodes(root).find((n) => n.props.testID === "workspace-chat-header-row")!;
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
    assert.equal(flat(target.props.style).minHeight, 44);
    // Wide bars: one flat control; the chevron follows the labels.
    assert.equal(flat(target.props.style).alignSelf, "flex-start");
    assert.equal(flat(target.props.style).borderRadius, undefined);
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

test("the agent chevron stays beside the name in every layout; floating bars make it the shared glass pill", () => {
  for (const floating of [false, true]) {
    const x = setup();
    x.h.mount(() =>
      x.ChatIdentity({
        agent: bot,
        name: "NyxBot",
        detail: "New chat",
        floating,
        compact: floating,
        onPress: () => {},
      }),
    );
    const target = x.h.value();
    // Floating bars: the pill is the shared glass button (one capsule around
    // avatar, title and chevron); otherwise a plain control.
    assert.equal(target.type, floating ? "FloatingButton" : "Pressable");
    // The contents are one fragment: avatar | labels | chevron, with no
    // flexible gap that would push the chevron to the far edge of the bar.
    const children = [target.props.children]
      .flat()
      .flatMap((n: any) => (n?.type === undefined ? [n.props.children].flat() : [n]))
      .filter(Boolean);
    assert.deepEqual(
      children.map((n: any) => n.type),
      ["AgentAvatar", "View", "ChevronDown"],
    );
    const labels = flat(children[1].props.style);
    assert.equal(labels.flex, undefined);
    assert.equal(labels.flexShrink, 1);
    if (floating) {
      // The glass hugs the pill and yields width; only the title truncates.
      const capsule = flat(target.props.capsuleStyle);
      assert.equal(capsule.alignSelf, "flex-start");
      assert.equal(capsule.maxWidth, "100%");
      assert.equal(capsule.minWidth, 0);
      const row = flat(target.props.style);
      assert.equal(row.flexDirection, "row");
      // No surface, radius or press styling of its own: the primitive's.
      assert.equal(row.backgroundColor, undefined);
      assert.equal(row.borderRadius, undefined);
      assert.equal(row.opacity, undefined);
      assert.equal(children[0].props.size, "lg");
    } else {
      const style = flat(target.props.style);
      assert.equal(style.alignSelf, "flex-start");
      assert.equal(style.maxWidth, "100%");
      assert.equal(style.backgroundColor, undefined);
    }
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
    assert.equal(bar(x.h.value()).type, "MotionView");
    assert.equal(bar(x.h.value()).props.entering, undefined, "no fade off the Duo");
    assert.equal(bar(x.h.value()).props.style.paddingTop, 62);
    const row = headerRow(x.h.value());
    assert.equal(flatStyle(row).flexDirection, "row");
    assert.equal(flatStyle(row).height, WORKSPACE_HEADER_ROW_HEIGHT);
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
    "../../i18n/appTranslation": { translateApp: (phrase: string) => phrase },
    "../../i18n/LanguageProvider": {
      useAppLanguage: () => {
        hookCalls++;
        return { t: (phrase: string) => phrase };
      },
    },
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
    assert.equal(hookCalls, 2);
    h.unmount();
    hookCalls = 0;
  }
});

test("narrow chat headers give identity a separate full-width row while retaining every action", () => {
  window = { width: 274, height: 610 };
  const x = setup();
  let picked = 0;
  x.h.mount(() => {
    const element = x.ChatHeaderFrame({ route: { name: "Assistant" }, options: {
      headerLeft: () => ({ type: "Menu", props: {} }),
      headerTitle: () => ({ type: "Identity", props: { onPress: () => picked++ } }),
      headerRight: () => ({ type: "AllActions", props: {} }),
    } });
    return (element.type as (props: any) => E)(element.props);
  });
  const identityRow = nodes(x.h.value()).find((node) => node.props.testID === "workspace-chat-identity-row");
  assert.ok(identityRow, "identity must not compete with the navigation controls at 274pt");
  assert.equal(nodes(headerRow(x.h.value())).some((node) => node.type === "Identity"), false);
  assert.ok(nodes(headerRow(x.h.value())).some((node) => node.type === "AllActions"));
  nodes(identityRow).find((node) => node.type === "Identity")!.props.onPress();
  assert.equal(picked, 1);
  x.h.unmount();
  window = { width: 402, height: 874 };
});

test("on the Duo (folded or unfolded) a chat has no top bar in rail mode and its own bar, fading in, in full screen; ordinary phones and iPads keep theirs", () => {
  const corner = { value: true };
  const rail = { value: true };
  const motion = { value: { inset: 66, fullScreen: false, progress: { value: 0 } } as any };
  let insets = { top: 0, left: 0, right: 83 };
  window = { width: 466, height: 678 };
  const h = hookHarness<E>({
    ...mocks,
    "react-native-safe-area-context": { useSafeAreaInsets: () => insets },
    "../../components/workspace/WorkspaceLayoutContext": {
      WorkspaceCornerSensorContext: corner,
      WorkspaceQuickRailContext: rail,
    },
    "../../components/workspace/workspaceFloatingInset": {
      WorkspaceRailMotionContext: motion,
    },
  });
  const { ChatHeaderFrame } = h.load(
    fileURLToPath(new URL("./ChatIdentityHeader.tsx", import.meta.url)),
  ) as { ChatHeaderFrame: (props: any) => E };
  const options = {
    headerLeft: () => ({ type: "Menu", props: {} }),
    headerTitle: () => ({ type: "Identity", props: {} }),
    headerRight: () => ({ type: "More", props: {} }),
  };
  h.mount(() => {
    const element = ChatHeaderFrame({ route: { name: "Assistant" }, options });
    return (element.type as (props: any) => E)(element.props);
  });
  try {
    // Rail mode, folded and unfolded alike: only the top inset, no row and
    // none of the bar's controls (they sit in the status column), so the
    // chat starts at the top: 68pt more height than the old 8–68pt bar.
    for (const [width, height, right] of [
      [466, 678, 83],
      [951, 669, 104],
    ] as const) {
      window = { width, height };
      insets = { top: 0, left: 0, right };
      h.render();
      const spacer = h.value();
      assert.equal(spacer.props.testID, "workspace-chat-header");
      assert.equal(spacer.props.style.height, insets.top);
      assert.equal(spacer.props.children, undefined);
      assert.equal(
        nodes(spacer).some((n) => ["Menu", "Identity", "More"].includes(n.type as string)),
        false,
      );
      // Full screen: the chat's own bar returns (as on an iPhone), fading
      // in: the flush row (8–68pt), its controls left of the status column,
      // content right under it.
      motion.value = { ...motion.value, fullScreen: true };
      h.render();
      const header = bar(h.value());
      assert.equal(header.props.entering, "fade-in");
      assert.equal(header.props.exiting, "fade-out");
      assert.equal(header.props.style.paddingTop, 8);
      assert.equal(header.props.style.paddingBottom, 0);
      assert.ok(flatStyle(headerRow(header)).paddingRight >= right);
      // Its bottom edge fades 24pt into the page (over the content).
      const fade = nodes(header).find((n) => n.props.testID === "workspace-chat-header-fade")!;
      assert.equal(fade.type, "LinearGradient");
      assert.equal(fade.props.pointerEvents, "none");
      assert.equal(JSON.stringify(fade.props.colors), JSON.stringify(["background", "background00"]));
      assert.deepEqual(
        { top: fade.props.style.top, height: fade.props.style.height },
        { top: "100%", height: 24 },
      );
      assert.deepEqual(
        [headerRow(header).props.children]
          .flat()
          .map((n: E) => (n.type === "View" ? (n.props.children as E).type : n.type)),
        ["Menu", "Identity", "More"],
      );
      motion.value = { ...motion.value, fullScreen: false };
    }
    motion.value = null;
    // Ordinary phones and landscape notches keep the bar at their safe-area
    // top, with the shared 12pt margin.
    rail.value = false;
    corner.value = false;
    h.render();
    assert.equal(
      nodes(h.value()).some((n) => n.props.testID === "workspace-chat-header-fade"),
      false,
      "no fade off the Duo",
    );
    window = { width: 852, height: 393 };
    insets = { top: 0, left: 59, right: 59 };
    h.render();
    assert.equal(bar(h.value()).props.style.paddingTop, 0);
    assert.equal(flatStyle(headerRow(h.value())).paddingLeft, 12 + 59);
    window = { width: 402, height: 874 };
    insets = { top: 62, left: 0, right: 0 };
    h.render();
    assert.equal(bar(h.value()).props.style.paddingTop, 62);
    const row = headerRow(h.value());
    assert.deepEqual(
      [row.props.children].flat().map((n: E) => (n.type === "View" ? (n.props.children as E).type : n.type)),
      ["Menu", "Identity", "More"],
    );
    // iPad (beside the persistent sidebar) floats the same glass controls
    // with the same 12pt margin and 8pt gap.
    window = { width: 1366, height: 1024 };
    insets = { top: 24, left: 0, right: 0 };
    h.render();
    assert.equal(flatStyle(headerRow(h.value())).paddingLeft, 12);
    assert.equal(flatStyle(headerRow(h.value())).gap, 8);
  } finally {
    window = { width: 402, height: 874 };
    h.unmount();
  }
});

test("P9 identity retains base native paragraph identity", () => {
  const source = readFileSync(
    new URL("./ChatIdentityHeader.tsx", import.meta.url),
    "utf8",
  );
  assert.doesNotMatch(source, /DynamicTypeText as Text/);
  assert.match(source, /import \{[^}]*\bText\b[^}]*\} from "react-native"/);
});

test("the compact identity keeps a fixed 32pt avatar and one tail-truncating title", () => {
  for (const props of [
    { agent: specialist, name: specialist.display_name },
    { members: [bot, specialist], name: "Launch crew" },
  ]) {
    const x = setup();
    x.h.mount(() =>
      x.ChatIdentity({
        ...props,
        compact: true,
        detail: "Release notes · 2h",
        status: "Running",
        onPress: () => {},
      }),
    );
    const all = nodes(x.h.value());
    const mark = all.find(
      (n) => n.type === "AgentAvatar" || n.type === "AgentAvatarStack",
    )!;
    assert.equal(mark.props.size, "lg");
    const lines = all.filter(
      (n) => n.type === "Text" && n.props.ellipsizeMode === "tail",
    );
    assert.equal(lines.length, 1, "the thread line is omitted");
    assert.equal(text(lines[0]), props.name);
    assert.ok(all.some((n) => n.type === "Text" && text(n) === "Running"));
    assert.ok(all.indexOf(mark) < all.indexOf(lines[0]!));
    x.h.unmount();
  }
});
