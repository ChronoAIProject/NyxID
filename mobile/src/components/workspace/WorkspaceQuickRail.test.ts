import assert from "node:assert/strict";
import test from "node:test";
import { fileURLToPath, URL } from "node:url";
import { hookHarness } from "../../features/assistant/assistantHookTestHarness";
import { translationTestMocks } from "../../i18n/translationTestHarness";
import { setAppTranslationLanguage, translateApp } from "../../i18n/appTranslation";
import { darkColors, lightColors } from "../../theme/mobileTheme";
import {
  QUICK_RAIL_BUTTON_GAP,
  QUICK_RAIL_SECTION_GAP,
  QUICK_RAIL_TAB_SIZE,
  QUICK_RAIL_WIDTH,
} from "./workspaceLayout";

type E = { type: unknown; props: Record<string, any> };
const nodes = (node: any): E[] =>
  node?.props
    ? [node, ...[node.props.children].flat(Infinity).flatMap(nodes)]
    : [];

/** The account's list as the hook returns it (the default until customised). */
const DEFAULT_QUICK_ACCESS = {
  items: ["new_chat", "notifications", "scan_qr", "recent_chats", "settings"].map(
    (id) => ({ kind: "destination", id }),
  ),
  customized: false,
};

function setup(
  notifications: { enabled: boolean; unread: number },
  mode: "light" | "dark" = "light",
  fullScreen = false,
  header: Record<string, any> | null = null,
  quickAccess: { items: Record<string, string>[]; customized: boolean } = DEFAULT_QUICK_ACCESS,
  sources: { agents?: unknown[]; crews?: unknown[] } = {},
) {
  const colors = mode === "light" ? lightColors : darkColors;
  const pressed: string[] = [];
  const indicatorActive: boolean[] = [];
  const requested: { agents: boolean; crews: boolean }[] = [];
  const h = hookHarness<E>({
    ...translationTestMocks,
    "react/jsx-runtime": {
      jsx: (type: unknown, props: unknown) => ({ type, props }),
      jsxs: (type: unknown, props: unknown) => ({ type, props }),
    },
    "react-native": {
      View: "View",
      Pressable: "Pressable",
      StyleSheet: { create: (v: unknown) => v, hairlineWidth: 0.5 },
      AppState: {
        currentState: "active",
        addEventListener: () => ({ remove() {} }),
      },
      findNodeHandle: (ref: unknown) => (ref ? 77 : null),
      ScrollView: "ScrollView",
    },
    "lucide-react-native": Object.fromEntries(
      [
        "Bell",
        "ChevronLeft",
        "Menu",
        "MessagesSquare",
        "MoreHorizontal",
        "ScanQrCode",
        "Settings",
        "SquarePen",
      ].map((name) => [name, name]),
    ),
    "./HorizontalArrowIcons": {
      ExpandHorizontalIcon: "ExpandHorizontalIcon",
      RestoreHorizontalIcon: "RestoreHorizontalIcon",
    },
    "../AgentAvatar": {
      AgentAvatar: "AgentAvatar",
      AgentAvatarStack: "AgentAvatarStack",
    },
    "../../theme/ThemeContext": {
      useTheme: () => ({ colors, mode }),
    },
    "../../theme/designTokens": { radius: { md: 8, xl: 16 } },
    "../../hooks/useNetworkStatus": {
      useNetworkStatus: () => ({ isConnected: true }),
    },
    "../../features/notifications/useNotifications": {
      useNotificationIndicator: (active: boolean) => {
        indicatorActive.push(active);
        return { ...notifications, needsDecision: 0, markers: {} };
      },
    },
    "../../features/quickAccess/useQuickAccess": {
      useQuickAccess: () => ({ state: quickAccess }),
    },
    "../../features/quickAccess/useQuickAccessSources": {
      useQuickAccessSources: (flags: { agents: boolean; crews: boolean }) => {
        requested.push(flags);
        return sources;
      },
    },
    "../../features/quickAccess/QuickAccessGlyph": {
      QuickAccessGlyph: "QuickAccessGlyph",
    },
  });
  const module = h.load(
    fileURLToPath(new URL("./WorkspaceQuickRail.tsx", import.meta.url)),
  ) as any;
  // Each entry opens through one callback with its saved item.
  const short: Record<string, string> = {
    new_chat: "new",
    notifications: "notifications",
    scan_qr: "scan",
    recent_chats: "recents",
    settings: "settings",
  };
  const actions = {
    onOpen: (item: { kind: string; id: string }) =>
      pressed.push(item.kind === "destination" ? (short[item.id] ?? item.id) : `${item.kind}:${item.id}`),
    onToggleFullScreen: () => pressed.push("toggle"),
  };
  h.mount(() => module.WorkspaceQuickRail({ ...actions, fullScreen, header }));
  return { h, module, pressed, indicatorActive, requested };
}

/** Every node, expanding the rail's small hook-free components (its marks). */
const expanded = (root: E): E[] =>
  nodes(root).flatMap((n) =>
    typeof n.type === "function" && (n.type as any).name === "RailGlyph"
      ? [n, ...expanded((n.type as any)(n.props))]
      : [n],
  );

// Every rail control renders through the shared glass primitive: the toggle
// is a FloatingButton and each shortcut a FloatingItem (RailItem is file-private;
// render its element to reach the primitive).
const pressables = (root: E): E[] =>
  nodes(root)
    .flatMap((n) =>
      typeof n.type === "function" ? nodes((n.type as any)(n.props)) : [n],
    )
    .filter((n) => n.type === "FloatingButton" || n.type === "FloatingItem");

const flat = (style: unknown) =>
  Object.assign({}, ...[style].flat(Infinity).filter(Boolean));
const capsules = (root: E) => {
  const rail = nodes(root).find(
    (n) => n.props.testID === "workspace-quick-rail-content",
  )!;
  const column = [rail.props.children].flat(Infinity).filter(Boolean) as E[];
  // Top to bottom: the toggle, a chat's header capsule (when there is one),
  // then the grouped shortcuts in their full-screen fade (RailShortcutsView).
  const [toggle] = column;
  const motion = column.find((n) => n.type === "RailShortcutsView")!;
  const headerCapsule = column.find((n) => typeof n.type === "function");
  const group = [motion.props.children].flat(Infinity).filter(Boolean)[0] as E;
  return { rail, column, toggle: toggle!, motion, group, headerCapsule };
};

test("grouped floating controls: the toggle in its own capsule, then one capsule holding New chat, Notifications, Scan QR, Recent chats and Settings", () => {
  const x = setup({ enabled: true, unread: 0 });
  const { rail, toggle, motion, group } = capsules(x.h.value());
  // One glass container groups the capsules (Apple's grouped glass); it is
  // the toolbar. They stay distinct at rest (spacing below the gap).
  assert.equal(rail.type, "FloatingGroup");
  assert.ok(rail.props.spacing < QUICK_RAIL_SECTION_GAP);
  assert.equal(rail.props.accessibilityRole, "toolbar");
  assert.equal(rail.props.accessibilityLabel, "Quick actions");
  assert.equal(rail.props.pointerEvents, "box-none");
  assert.equal(flat(rail.props.style).gap, QUICK_RAIL_SECTION_GAP);
  assert.equal(flat(rail.props.style).width, QUICK_RAIL_WIDTH);
  // Two capsules of one family, the toggle first (at the shared toggle spot):
  // the toggle is the shared FloatingButton (its own capsule), the shortcuts
  // share one FloatingCapsule, both 44pt wide and fully rounded, both in the
  // primitive's surface (no surface keys of their own).
  assert.equal(toggle.type, "FloatingButton");
  assert.equal(toggle.props.testID, "quick-rail-expand");
  assert.equal(motion.type, "RailShortcutsView");
  assert.equal(motion.props.pointerEvents, "box-none");
  // Liquid glass hides itself; only the fallback surface fades its wrapper.
  assert.equal(motion.props.fadeOpacity, false);
  assert.equal(group.props.visible, true);
  assert.equal(group.type, "FloatingCapsule");
  assert.equal(group.props.testID, "quick-rail-group");
  assert.equal(group.props.glass, "rail", "Duo rail glass only");
  for (const capsule of [flat(toggle.props.capsuleStyle), flat(group.props.style)]) {
    assert.equal(capsule.width, QUICK_RAIL_TAB_SIZE);
    assert.equal(capsule.borderRadius, QUICK_RAIL_TAB_SIZE / 2);
    assert.equal(capsule.backgroundColor, undefined);
    assert.equal(capsule.borderWidth, undefined);
  }
  assert.equal(flat(group.props.style).gap, QUICK_RAIL_BUTTON_GAP);
  assert.equal(flat(group.props.style).height, undefined, "hugs its buttons");
  assert.deepEqual(
    pressables(toggle).map((b) => b.props.accessibilityLabel),
    ["Full screen"],
  );
  assert.deepEqual(
    pressables(group).map((b) => b.props.accessibilityLabel),
    ["New chat", "Notifications", "Scan QR code", "Recent chats", "Settings"],
  );
  // Shortcuts are plain items in the one capsule, not separate buttons.
  assert.equal(pressables(group).every((n) => n.type === "FloatingItem"), true);
  for (const button of pressables(x.h.value())) button.props.onPress();
  assert.deepEqual(x.pressed, ["toggle", "new", "notifications", "scan", "recents", "settings"]);
  assert.deepEqual(x.indicatorActive.at(-1), true);
  x.h.unmount();
});

test("controls inside the grouped capsule are FloatingItems (44pt, no chrome, a round pressed highlight) with the bell's unread dot", () => {
  for (const mode of ["light", "dark"] as const) {
    const x = setup({ enabled: true, unread: 2 }, mode);
    const { group } = capsules(x.h.value());
    for (const item of pressables(group)) {
      assert.equal(item.type, "FloatingItem");
      // No surface, border or press styling of its own: the primitive
      // supplies the 44pt target and the highlight.
      assert.equal(item.props.style, undefined);
      assert.equal(item.props.disabled, false);
    }
    const dot = expanded(x.h.value()).find(
      (n) => n.props.testID === "quick-rail-notifications-dot",
    )!;
    assert.equal(flat(dot.props.style).position, "absolute");
    x.h.unmount();
  }
});

/** WCAG 2.1 relative luminance of an sRGB triple. */
function luminance([r, g, b]: number[]) {
  const [R, G, B] = [r!, g!, b!].map((v) => {
    const c = v / 255;
    return c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
  });
  return 0.2126 * R! + 0.7152 * G! + 0.0722 * B!;
}
const hex = (h: string) => [1, 3, 5].map((i) => parseInt(h.slice(i, i + 2), 16));
const rgba = (s: string) => {
  const [r, g, b, a] = s.match(/[\d.]+/g)!.map(Number);
  return { rgb: [r!, g!, b!], alpha: a! };
};
function contrast(a: number[], b: number[]) {
  const [hi, lo] = [luminance(a), luminance(b)].sort((p, q) => q - p);
  return (hi! + 0.05) / (lo! + 0.05);
}

test("icons on the floating-control surface keep at least 4.5:1 in light and dark", () => {
  for (const [mode, colors] of [
    ["light", lightColors],
    ["dark", darkColors],
  ] as const) {
    const ratio = contrast(hex(colors.textPrimary), hex(colors.floatingControlBg));
    assert.ok(ratio >= 4.5, `${mode}: ${ratio.toFixed(2)}:1`);
  }
  assert.equal(Math.round(contrast([0, 0, 0], [255, 255, 255])), 21);
});

test("Notifications follows the feature check and carries one unread label and dot", () => {
  const off = setup({ enabled: false, unread: 4 });
  assert.deepEqual(
    pressables(capsules(off.h.value()).group).map((b) => b.props.accessibilityLabel),
    ["New chat", "Scan QR code", "Recent chats", "Settings"],
    "absent when assistant:notifications is off for this person",
  );
  off.h.unmount();
  const read = setup({ enabled: true, unread: 0 });
  assert.equal(
    expanded(read.h.value()).some((n) => n.props.testID === "quick-rail-notifications-dot"),
    false,
  );
  read.h.unmount();
  const unread = setup({ enabled: true, unread: 3 });
  const bell = pressables(capsules(unread.h.value()).group)[1]!;
  assert.equal(bell.props.accessibilityLabel, "Notifications, 3 unread");
  assert.ok(expanded(unread.h.value()).some((n) => n.props.testID === "quick-rail-notifications-dot"));
  unread.h.unmount();
});

test("the unread label is one translated template in every app language", () => {
  try {
    for (const language of ["zh-CN", "zh-TW"] as const) {
      setAppTranslationLanguage(language);
      const label = translateApp("Notifications, {{unread}} unread", { unread: 3 });
      assert.notEqual(label, "Notifications, 3 unread");
      assert.ok(label.includes("3"));
      for (const phrase of ["Full screen", "Show quick actions", "Recent chats"])
        assert.notEqual(translateApp(phrase), phrase, `${language}: ${phrase}`);
    }
  } finally {
    setAppTranslationLanguage("en");
  }
});

test("full screen keeps the same toggle capsule (now Show quick actions) and fades only the grouped capsule", () => {
  const x = setup({ enabled: true, unread: 1 }, "light", true);
  const { toggle, motion, group } = capsules(x.h.value());
  // The toggle: same capsule, same spot, expanded icon, fully visible.
  assert.equal(toggle.props.visible, undefined, "never hidden");
  const [restore] = pressables(toggle);
  assert.equal(restore!.type, "FloatingButton");
  assert.equal(restore!.props.accessibilityLabel, "Show quick actions");
  // The outward-pointing horizontal arrows ( ←  → ): the expanded state.
  assert.ok(nodes(toggle).some((n) => n.type === "ExpandHorizontalIcon"));
  assert.equal(nodes(toggle).some((n) => n.type === "RestoreHorizontalIcon"), false);
  restore!.props.onPress();
  assert.deepEqual(x.pressed, ["toggle"]);
  // The grouped capsule stays mounted and fades out on the UI thread
  // (RailShortcutsView); it takes no touches and leaves VoiceOver.
  assert.equal(motion.props.pointerEvents, "none");
  assert.equal(motion.props.accessibilityElementsHidden, true);
  assert.equal(motion.props.importantForAccessibility, "no-hide-descendants");
  // Its glass dissolves natively and its icons fade out.
  assert.equal(group.props.visible, false);
  const buttons = pressables(group);
  assert.equal(buttons.length, 5);
  for (const button of buttons) {
    // Disabled: no highlight and no touches while hidden.
    assert.equal(button.props.disabled, true);
    // The icon fades on the UI thread with the content's width; it stays
    // mounted so the capsule keeps its size.
    const icon = [button.props.children].flat().filter(Boolean)[0];
    assert.equal(icon.type, "RailShortcutIcon");
  }
  x.h.unmount();
});

/** Render the rail's header capsule component (it has its own hooks). */
/** Render a header capsule element: its motion wrapper and glass capsule. */
function renderCapsuleElement(x: ReturnType<typeof setup>, element: E) {
  x.h.unmount();
  // Unwrap component layers down to the capsule's motion wrapper.
  x.h.mount(() => {
    let current = element;
    while (typeof current.type === "function")
      current = (current.type as (props: unknown) => E)(current.props);
    return current;
  });
  const wrapper = x.h.value();
  const capsule = [wrapper.props.children].flat(Infinity).filter(Boolean)[0] as E;
  return { wrapper, capsule };
}
function renderHeaderCapsule(x: ReturnType<typeof setup>) {
  const element = capsules(x.h.value()).headerCapsule;
  return element ? renderCapsuleElement(x, element).capsule : undefined;
}
/** Each item's glyph, inside its icon fade. */
const glyphOf = (item: E) => {
  const fade = [item.props.children].flat()[0] as E;
  assert.equal(fade.type, "RailShortcutIcon");
  return [fade.props.children].flat()[0] as E;
};

function chatHeader(presses: string[], focus: [unknown, boolean][] = []) {
  return {
    leading: {
      role: "menu",
      label: "Open navigation",
      onPress: () => presses.push("menu"),
      registerFocus: (target: () => unknown, active: boolean) =>
        focus.push([target(), active]),
    },
    identity: {
      label: "NyxBot. Choose agent",
      agent: { id: "nyxbot", name: "NyxBot", kind: "nyxbot" },
      onPress: () => presses.push("agent"),
    },
    more: { label: "Chat options", onPress: () => presses.push("more") },
  };
}

test("on a chat the column is the toggle, the shortcuts, then its bar's controls; full screen hides the shortcuts and the header capsule", () => {
  const presses: string[] = [];
  const focus: [unknown, boolean][] = [];
  const header = chatHeader(presses, focus);
  const modes = [false, true].map((fullScreen) => {
    const x = setup({ enabled: true, unread: 0 }, "light", fullScreen, header);
    const result = { ...capsules(x.h.value()), x };
    return result;
  });
  for (const [index, mode] of modes.entries()) {
    const fullScreen = index === 1;
    // One glass container, 12pt between capsules, in this order.
    assert.equal(mode.rail.type, "FloatingGroup");
    assert.equal(flat(mode.rail.props.style).gap, QUICK_RAIL_SECTION_GAP);
    assert.deepEqual(
      mode.column.map((n) => (n === mode.headerCapsule ? "header" : n.type)),
      ["FloatingButton", "RailShortcutsView", "header"],
    );
    assert.ok(
      nodes(mode.toggle).some(
        (n) => n.type === (fullScreen ? "ExpandHorizontalIcon" : "RestoreHorizontalIcon"),
      ),
    );
    // Only the shortcuts change: shown in rail mode, hidden in full screen.
    assert.equal(mode.group.props.visible, !fullScreen);
    assert.equal(mode.motion.props.pointerEvents, fullScreen ? "none" : "box-none");
  }
  // The column's header capsule is the same element in both modes, shown in
  // rail mode and hidden (cross-faded to the top header) in full screen.
  // (Each setup loads its own module copy, so compare the component by name.)
  assert.equal(
    (modes[0]!.headerCapsule!.type as { name: string }).name,
    (modes[1]!.headerCapsule!.type as { name: string }).name,
  );
  assert.deepEqual(
    Object.keys(modes[0]!.headerCapsule!.props).sort(),
    ["controls", "shown"],
  );
  assert.equal(modes[0]!.headerCapsule!.props.controls, modes[1]!.headerCapsule!.props.controls);
  assert.deepEqual(
    modes.map((mode) => mode.headerCapsule!.props.shown),
    [true, false],
  );
  assert.deepEqual(
    modes.map((mode) => mode.toggle.props.capsuleGlass === "railOpaque"),
    [false, true],
  );
  for (const mode of modes) mode.x.h.unmount();
});

test("the header capsule: live in rail mode; in full screen (the chat's bar returns) it fades out, takes no touches and leaves VoiceOver", () => {
  for (const fullScreen of [false, true]) {
    const presses: string[] = [];
    const focus: [unknown, boolean][] = [];
    const header = chatHeader(presses, focus);
    const x = setup({ enabled: true, unread: 0 }, "dark", fullScreen, header);
    const shown = !fullScreen;
    const { wrapper, capsule } = renderCapsuleElement(x, capsules(x.h.value()).headerCapsule!);
    // Its wrapper fades with the shortcuts on the fallback surface only
    // (glass hides itself).
    assert.equal(wrapper.type, "RailShortcutsView");
    assert.equal(wrapper.props.fadeOpacity, false);
    assert.equal(wrapper.props.pointerEvents, shown ? "box-none" : "none");
    assert.equal(wrapper.props.accessibilityElementsHidden, !shown);
    assert.equal(capsule.type, "FloatingCapsule");
    assert.equal(capsule.props.testID, "quick-rail-header");
    assert.equal(capsule.props.visible, shown);
    assert.equal(capsule.props.glass, "rail", "Duo rail glass");
    // Laid out in the column (never absolutely positioned over another slot).
    const style = flat(capsule.props.style);
    assert.equal(style.position, undefined);
    assert.equal(style.width, QUICK_RAIL_TAB_SIZE);
    assert.equal(style.borderRadius, QUICK_RAIL_TAB_SIZE / 2);
    assert.equal(style.gap, QUICK_RAIL_BUTTON_GAP);
    const items = [capsule.props.children].flat(Infinity).filter(Boolean) as E[];
    assert.deepEqual(
      items.map((item) => [item.type, item.props.accessibilityLabel]),
      [
        ["FloatingItem", "Open navigation"],
        ["FloatingItem", "NyxBot. Choose agent"],
        ["FloatingItem", "Chat options"],
      ],
    );
    const glyphs = items.map(glyphOf);
    assert.deepEqual(glyphs.map((g) => g.type), ["Menu", "AgentAvatar", "MoreHorizontal"]);
    assert.equal(glyphs[1]!.props.agent.id, "nyxbot");
    assert.equal(glyphs[1]!.props.size, "lg");
    for (const item of items) assert.equal(item.props.disabled, !shown);
    if (shown) {
      for (const item of items) item.props.onPress();
      assert.deepEqual(presses, ["menu", "agent", "more"]);
      // Its Menu is the chat's focus-restoration target in rail mode.
      assert.equal(focus.at(-1)![1], true);
    } else assert.equal(focus.length, 0, "hidden: the chat's bar owns focus");
    x.h.unmount();
  }
});

test("a crew's column shows Back and the crew's avatars with disabled controls kept; other pages get no header capsule and the shortcuts follow the toggle", () => {
  const crew = {
    leading: { role: "back", label: "Back", onPress() {}, disabled: true },
    identity: {
      label: "Launch crew. Choose agent",
      members: [
        { id: "a", name: "NyxBot", kind: "nyxbot" },
        { id: "b", name: "Researcher", kind: "specialist" },
      ],
      onPress() {},
    },
    more: { label: "Crew options", onPress() {}, disabled: true },
  };
  const x = setup({ enabled: true, unread: 0 }, "light", false, crew);
  const capsule = renderHeaderCapsule(x)!;
  const items = [capsule.props.children].flat(Infinity).filter(Boolean) as E[];
  const glyphs = items.map(glyphOf);
  assert.deepEqual(glyphs.map((g) => g.type), ["ChevronLeft", "AgentAvatarStack", "MoreHorizontal"]);
  assert.equal(glyphs[1]!.props.agents.length, 2);
  assert.equal(glyphs[1]!.props.max, 2);
  assert.equal(glyphs[1]!.props.size, "xs", "two marks fit the 44pt capsule");
  assert.equal(glyphs[1]!.props.surface, "floating-surface", "rings cut out against the glass");
  assert.deepEqual(items.map((item) => item.props.disabled), [true, false, true]);
  x.h.unmount();
  // A page with its own bar (Notifications, Automations…): no header capsule;
  // the shortcuts sit right under the toggle.
  for (const fullScreen of [false, true]) {
    const page = setup({ enabled: true, unread: 0 }, "light", fullScreen, null);
    const { column, headerCapsule } = capsules(page.h.value());
    assert.equal(headerCapsule, undefined);
    assert.deepEqual(column.map((n) => n.type), ["FloatingButton", "RailShortcutsView"]);
    page.h.unmount();
  }
});

const AGENT = "6f9619ff-8b86-d011-b42d-00c04fc964ff";
const GONE_AGENT = "11111111-2222-4333-8444-555555555555";
const DESTROYED = "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee";
const CREW = "nyxg-0123456789abcdef0123456789abcdef";
const GONE_CREW = "nyxg-ffffffffffffffffffffffffffffffff";

/** The shortcuts capsule's items: [spoken label, the mark it shows]. */
const shortcutItems = (root: E) =>
  pressables(capsules(root).group).map((item) => {
    const mark = expanded(item).find((n) => n.type === "QuickAccessGlyph");
    return [item.props.accessibilityLabel, mark?.props.entry.kind];
  });

test("the shortcuts capsule holds the account's quick access in order: destinations, pinned agents and crews (avatars); deleted ones are skipped", () => {
  const quick = {
    customized: true,
    items: [
      { kind: "destination", id: "machines" },
      { kind: "agent", id: AGENT },
      { kind: "agent", id: GONE_AGENT },
      { kind: "agent", id: DESTROYED },
      { kind: "crew", id: CREW },
      { kind: "crew", id: GONE_CREW },
      { kind: "destination", id: "terminal_login_code" },
    ],
  };
  const sources = {
    agents: [
      { id: AGENT, name: "researcher", display_name: "Research", kind: "specialist", status: "idle" },
      { id: DESTROYED, name: "old", kind: "specialist", status: "destroyed" },
    ],
    crews: [{ id: CREW, name: "Launch crew", members: [] }],
  };
  const x = setup({ enabled: true, unread: 0 }, "light", false, null, quick, sources);
  // Agents and crews load only because the list pins them.
  assert.equal(JSON.stringify(x.requested.at(-1)), JSON.stringify({ agents: true, crews: true }));
  assert.deepEqual(shortcutItems(x.h.value()), [
    ["Machines", "destination"],
    ["Chat with Research", "agent"],
    ["Open crew Launch crew", "crew"],
    ["Terminal login code", "destination"],
  ]);
  // A pinned agent shows its avatar; a crew its members.
  const marks = expanded(capsules(x.h.value()).group).filter(
    (n) => n.type === "QuickAccessGlyph",
  );
  assert.equal(marks[1]!.props.entry.agent.id, AGENT);
  assert.equal(marks[2]!.props.entry.members.length, 0);
  for (const item of pressables(capsules(x.h.value()).group)) item.props.onPress();
  assert.deepEqual(x.pressed, [
    "machines",
    `agent:${AGENT}`,
    `crew:${CREW}`,
    "terminal_login_code",
  ]);
  x.h.unmount();
  // Until the lists load, pinned agents and crews are simply not shown yet.
  const loading = setup({ enabled: true, unread: 0 }, "light", false, null, quick, {});
  assert.deepEqual(
    shortcutItems(loading.h.value()).map(([label]) => label),
    ["Machines", "Terminal login code"],
  );
  loading.h.unmount();
});

test("never customised (or before the list loads): the default five, with no agent or crew lookups", () => {
  const x = setup({ enabled: true, unread: 0 });
  assert.equal(JSON.stringify(x.requested.at(-1)), JSON.stringify({ agents: false, crews: false }));
  assert.deepEqual(
    shortcutItems(x.h.value()).map(([label]) => label),
    ["New chat", "Notifications", "Scan QR code", "Recent chats", "Settings"],
  );
  x.h.unmount();
  // An empty list is a choice: no shortcuts capsule at all.
  const empty = setup({ enabled: true, unread: 0 }, "light", false, null, {
    items: [],
    customized: true,
  });
  const rail = nodes(empty.h.value()).find(
    (n) => n.props.testID === "workspace-quick-rail-content",
  )!;
  assert.equal(
    [rail.props.children].flat(Infinity).some((n: any) => n?.type === "RailShortcutsView"),
    false,
  );
  empty.h.unmount();
});

test("a list taller than the column scrolls inside its glass capsule (no indicator); otherwise the capsule hugs its items", () => {
  const twelve = {
    customized: true,
    items: [
      "new_chat",
      "notifications",
      "scan_qr",
      "terminal_login_code",
      "recent_chats",
      "automations",
      "machines",
      "plugins",
      "activity",
      "create_agent",
      "settings",
      "agents",
    ].map((id) => ({ kind: "destination", id })),
  };
  const header = {
    leading: { role: "menu", label: "Open navigation", onPress() {} },
    identity: { label: "NyxBot. Choose agent", agent: { id: "n", name: "NyxBot", kind: "nyxbot" }, onPress() {} },
    more: { label: "Chat options", onPress() {} },
  };
  // The folded Duo's column: 196pt to the composer's 640pt bottom is 444pt.
  for (const [withHeader, height] of [
    [true, 444 - 44 - 12 - 12 - (3 * 44 + 2 * 4)], // below it, the ☰/agent/⋯ capsule
    [false, 444 - 44 - 12],
  ] as const) {
    const x = setup({ enabled: true, unread: 0 }, "light", false, withHeader ? header : null, twelve);
    // Before the column is measured the capsule hugs.
    assert.equal(flat(capsules(x.h.value()).group.props.style).height, undefined);
    const column = nodes(x.h.value()).find((n) => n.props.testID === "quick-rail-column")!;
    assert.equal(column.props.pointerEvents, "box-none");
    column.props.onLayout({ nativeEvent: { layout: { height: 444 } } });
    x.h.render();
    const group = capsules(x.h.value()).group;
    // The glass capsule keeps its place and takes the room left; its items
    // (12 × 44pt + 11 × 4pt = 572pt) scroll inside it.
    assert.equal(flat(group.props.style).height, height);
    const scroll = [group.props.children].flat(Infinity)[0] as E;
    assert.equal(scroll.type, "ScrollView");
    assert.equal(scroll.props.showsVerticalScrollIndicator, false);
    assert.equal(pressables(scroll).length, 12);
    x.h.unmount();
  }
  // The default five fit without scrolling above the aligned chat controls.
  const x = setup({ enabled: true, unread: 0 }, "light", false, header);
  const column = nodes(x.h.value()).find((n) => n.props.testID === "quick-rail-column")!;
  column.props.onLayout({ nativeEvent: { layout: { height: 444 } } });
  x.h.render();
  const group = capsules(x.h.value()).group;
  assert.equal(flat(group.props.style).height, undefined);
  assert.equal(
    [group.props.children].flat(Infinity).some((n: any) => n?.type === "ScrollView"),
    false,
  );
  x.h.unmount();
});
