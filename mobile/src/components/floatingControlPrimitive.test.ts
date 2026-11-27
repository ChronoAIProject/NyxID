import assert from "node:assert/strict";
import { existsSync, readdirSync, readFileSync } from "node:fs";
import { join, relative } from "node:path";
import test from "node:test";
import { fileURLToPath, URL } from "node:url";

/*
 * One floating-control style everywhere: every floating control renders
 * through the primitive in FloatingControl.tsx (FloatingButton for a control
 * in its own capsule, FloatingItem inside a shared FloatingCapsule): liquid
 * glass with one theme tint, style and colour scheme, or the theme surface
 * where glass is unavailable. Component tests check each control's rendered
 * type; this keeps a second glass source or a one-off floating style from
 * creeping back in.
 */
const SRC = fileURLToPath(new URL("..", import.meta.url));
const read = (path: string) => readFileSync(join(SRC, path), "utf8");

function sources(dir: string): string[] {
  return readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const path = join(dir, entry.name);
    return entry.isDirectory()
      ? sources(path)
      : /\.tsx?$/.test(entry.name) && !/(\.test\.|Harness)/.test(entry.name)
        ? [path]
        : [];
  });
}

test("glass and the floating-control surface have one source: the primitive", () => {
  const glass = sources(SRC)
    .filter((file) =>
      /from "expo-glass-effect"/.test(readFileSync(file, "utf8")),
    )
    .map((file) => relative(SRC, file));
  assert.deepEqual(glass, ["components/FloatingControl.tsx"]);
  const blur = sources(SRC)
    .filter((file) => /from "expo-blur"/.test(readFileSync(file, "utf8")))
    .map((file) => relative(SRC, file));
  assert.deepEqual(blur, []);
  const surfaceUsers = sources(SRC)
    .filter((file) =>
      /floatingControlSurface\(|floatingControlGlassTint\(/.test(
        readFileSync(file, "utf8"),
      ),
    )
    .map((file) => relative(SRC, file))
    .sort();
  assert.deepEqual(surfaceUsers, [
    "components/FloatingControl.tsx",
    "components/workspace/workspaceFloatingSurface.ts",
  ]);
  for (const removed of [
    "components/GlassBackdrop.tsx",
    "components/BlurBackButton.tsx",
    "components/GlassCapsule.tsx",
  ])
    assert.equal(existsSync(join(SRC, removed)), false, removed);
});

test("every floating control renders through the floating-control primitive", () => {
  const controls: [string, RegExp[]][] = [
    // Header ☰ / Back (and the sidebar toggle beside the persistent sidebar).
    ["app/WorkspaceHeader.tsx", [/<FloatingButton \{\.\.\.controlProps\}>/]],
    // New chat, More and crew options.
    [
      "features/assistant/ChatSurface.tsx",
      [/if \(floating\)\s+return \(\s+<FloatingButton/],
    ],
    // The agent / crew identity pill.
    [
      "features/assistant/ChatIdentityHeader.tsx",
      [/return floating \? \(\s+<FloatingButton/],
    ],
    // The ordinary bar's bell, when notifications are enabled.
    ["features/notifications/NotificationBell.tsx", [/<FloatingButton/]],
    // The rail: the toggle (and restore) button, the grouped capsule and,
    // in full screen on a chat, the column header capsule.
    [
      "components/workspace/WorkspaceQuickRail.tsx",
      [
        /<FloatingButton\s+testID="quick-rail-expand"/,
        /<FloatingCapsule\s+testID="quick-rail-group"/,
        /<FloatingCapsule\s+testID="quick-rail-header"/,
        /<FloatingItem/,
      ],
    ],
    // The legal pages' floating Back.
    ["components/FloatingBackButton.tsx", [/<FloatingButton/]],
    // An edit page's commit action (Save) on workspace and sheet bars.
    ["components/settings/SettingsHeaderButton.tsx", [/<FloatingButton/]],
  ];
  for (const [file, patterns] of controls) {
    const source = read(file);
    assert.match(source, /from "[./]+(components\/)?FloatingControl"/, file);
    for (const pattern of patterns)
      assert.match(source, pattern, `${file}: ${pattern}`);
    // No floating fill, border or shadow of their own: the primitive's
    // surface is the only treatment (the same for header and rail).
    assert.doesNotMatch(
      source,
      /shadowRadius|shadowOpacity|floatingControlBg/,
      file,
    );
  }
});

test("narrow and wide bars float the same glass controls", () => {
  // The trailing chat actions share one floating surface at all widths.
  const actions = read("features/assistant/ChatHeaderActions.tsx");
  assert.match(actions, /<ChatHeaderMenu/);
  assert.equal(
    actions.match(/<FloatingItem\b/g)?.length,
    3,
    "two private controls and one crew control",
  );
  const menu = read("features/assistant/ChatHeaderMenu.tsx");
  assert.equal(
    menu.match(/<FloatingCapsule\b/g)?.length,
    2,
    "source and overlay share the floating surface",
  );
  // The identity pill floats (compact) and crew options float everywhere.
  assert.match(
    read("features/assistant/AssistantScreen.tsx"),
    /<ChatIdentity\s+compact=\{quickRail\}\s+floating\s/,
  );
  const group = read("features/groups/GroupChatScreen.tsx");
  assert.doesNotMatch(group, /floating=\{/);
  assert.match(group, /<CrewHeaderActions/);
  assert.equal(group.match(/^\s+floating$/gm)!.length, 1);
  // ☰ / Back float on every bar, slab and sheet alike; only a sheet's own
  // Close stays plain.
  assert.match(read("app/WorkspaceHeader.tsx"), /const floating = !close;/);
});

test("every other workspace bar floats the chat bar's glass controls: Menu/Back, commit actions, shared spacing", () => {
  // Every place that puts a control in a bar (navigation `headerLeft` /
  // `headerRight`) is listed here with the floating control it renders, so a
  // new page cannot add a plain bar button unnoticed.
  const producers: Record<string, RegExp> = {
    // Menu/Back on every page and sheet page (Close is the one plain control).
    "app/WorkspaceHeader.tsx": /<FloatingButton \{\.\.\.controlProps\}>/,
    // The chat and crew bars (identity pill, New chat / More, crew
    // options).
    "features/assistant/AssistantScreen.tsx":
      /<ChatHeaderActions[\s\S]*<ChatIdentity\s+compact=\{quickRail\}\s+floating/,
    "features/groups/GroupChatScreen.tsx": /<CrewHeaderActions/,
    "features/notifications/NotificationsScreen.tsx":
      /<WorkspaceHeaderVisibility>\s*<FloatingButton[\s\S]*?<Settings2/,
    // Edit pages' Save, in the workspace and in the Settings sheet.
    "features/assistant/AssistantDetailsScreen.tsx": /<SettingsHeaderButton/,
    "features/agents/AgentProfileEditScreen.tsx": /<SettingsHeaderButton/,
    "features/agents/AgentAvatarEditScreen.tsx": /<SettingsHeaderButton/,
    "components/settings/SettingsList.tsx":
      /headerRight: \(\) => \(\s*<SettingsHeaderButton/,
  };
  const found = sources(SRC)
    .filter((file) =>
      /\bheader(Left|Right)\s*[:=(]|\bheader(Left|Right),/.test(
        readFileSync(file, "utf8"),
      ),
    )
    .map((file) => relative(SRC, file))
    // The bar renderers only call the options they are given.
    .filter(
      (file) =>
        ![
          "components/workspace/WorkspacePageHeader.tsx",
          "components/workspace/WorkspaceDrawerHeader.tsx",
          "features/assistant/ChatIdentityHeader.tsx",
        ].includes(file),
    )
    .sort();
  assert.deepEqual(found, Object.keys(producers).sort());
  for (const [file, pattern] of Object.entries(producers))
    assert.match(read(file), pattern, file);
  // The commit action is the floating pill everywhere: no plain Pressable or
  // bare spinner of its own (the spinner shows inside the capsule).
  const action = read("components/settings/SettingsHeaderButton.tsx");
  assert.doesNotMatch(action, /<Pressable/);
  assert.match(action, /busy \? \(\s*<ActivityIndicator/);
  const sheetAction = read("components/settings/SettingsList.tsx").match(
    /export function useSettingsHeaderAction[\s\S]*?\n\}\n/,
  )![0];
  assert.doesNotMatch(sheetAction, /<Pressable|<ActivityIndicator/);
  // Every bar renderer shares the chat bar's row, 12pt margins and 8pt gaps.
  for (const file of [
    "components/workspace/WorkspacePageHeader.tsx",
    "components/workspace/WorkspaceDrawerHeader.tsx",
    "features/assistant/ChatIdentityHeader.tsx",
  ]) {
    const source = read(file);
    assert.match(source, /WORKSPACE_HEADER_ROW/, file);
    assert.match(source, /FLOATING_HEADER_MARGIN/, file);
    assert.match(source, /gap: FLOATING_HEADER_GAP/, file);
  }
});

/** The surface keys a floating control draws (its fill, edge and lift). */
const SURFACE_KEYS = [
  "backgroundColor",
  "borderColor",
  "borderWidth",
  "shadowColor",
  "shadowOffset",
  "shadowOpacity",
  "shadowRadius",
  "elevation",
] as const;

test("header buttons, the identity pill and both rail capsules resolve to the same glass (and the same fallback surface) in light and dark", async () => {
  const { hookHarness } = await import(
    "../features/assistant/assistantHookTestHarness"
  );
  const { darkColors, lightColors } = await import("../theme/mobileTheme");
  // The theme pair: the colours the header buttons resolved to (#F9F9F9 on
  // the white page, #353539 on the dark one), now explicit tokens.
  assert.equal(lightColors.floatingControlBg, "#F9F9F9");
  assert.equal(darkColors.floatingControlBg, "#353539");
  for (const [liquidGlass, mode, colors] of [
    [true, "light", lightColors],
    [true, "dark", darkColors],
    [false, "light", lightColors],
    [false, "dark", darkColors],
  ] as const) {
    const h = hookHarness<any>({
      "react/jsx-runtime": {
        jsx: (type: unknown, props: unknown) => ({ type, props }),
        jsxs: (type: unknown, props: unknown) => ({ type, props }),
      },
      "react-native": {
        View: "View",
        Pressable: "Pressable",
        StyleSheet: { create: (v: unknown) => v, hairlineWidth: 0.5 },
      },
      "expo-glass-effect": {
        GlassView: "GlassView",
        GlassContainer: "GlassContainer",
        isLiquidGlassAvailable: () => liquidGlass,
      },
      "../theme/ThemeContext": { useTheme: () => ({ mode, colors }) },
    });
    const ui = h.load(join(SRC, "components/FloatingControl.tsx")) as any;
    const surfaceOf = (element: any) => {
      // FloatingButton → FloatingCapsule → the native view.
      let node = element;
      while (typeof node.type === "function") node = node.type(node.props);
      const style = Object.assign(
        {},
        ...[node.props.style].flat(Infinity).filter(Boolean),
      );
      return JSON.stringify(
        liquidGlass
          ? {
              type: node.type,
              tintColor: node.props.tintColor,
              glassEffectStyle: node.props.glassEffectStyle,
              colorScheme: node.props.colorScheme,
              isInteractive: node.props.isInteractive,
              surface: SURFACE_KEYS.map((key) => style[key]),
            }
          : { type: node.type, surface: SURFACE_KEYS.map((key) => style[key]) },
      );
    };
    const surfaces = {
      // ☰ / Back, More, New chat, the bell, crew options.
      headerButton: surfaceOf(ui.FloatingButton({ children: "Icon" })),
      // The identity pill (its capsule hugs the pill).
      identityPill: surfaceOf(
        ui.FloatingButton({
          capsuleStyle: {
            alignSelf: "flex-start",
            maxWidth: "100%",
            minWidth: 0,
          },
          children: "Pill",
        }),
      ),
      // The rail's toggle and grouped capsule (their layout styles).
      railToggle: surfaceOf(
        ui.FloatingButton({
          capsuleStyle: { width: 44, borderRadius: 22 },
          children: "Icon",
        }),
      ),
      railGroup: surfaceOf(
        ui.FloatingCapsule({
          style: [
            { width: 44, borderRadius: 22 },
            { alignItems: "center", gap: 4 },
          ],
          children: "Items",
        }),
      ),
    };
    const fallback = SURFACE_KEYS.map(
      (key) =>
        (
          ({
            backgroundColor: colors.floatingControlBg,
            borderColor: colors.floatingControlBorder,
            borderWidth: 0.5,
            shadowColor: colors.shadowColor,
            ...(mode === "light"
              ? {
                  shadowOffset: { width: 0, height: 2 },
                  shadowOpacity: 1,
                  shadowRadius: 10,
                  elevation: 3,
                }
              : {}),
          }) as Record<string, unknown>
        )[key],
    );
    const expected = JSON.stringify(
      liquidGlass
        ? {
            type: "GlassView",
            // The header-button token at 60% alpha, style 'regular', the
            // app's colour scheme, interactive; no opaque surface on top.
            tintColor: `${colors.floatingControlBg}99`,
            glassEffectStyle: {
              style: "regular",
              animate: true,
              animationDuration: 0.25,
            },
            colorScheme: mode,
            isInteractive: true,
            surface: SURFACE_KEYS.map(() => undefined),
          }
        : { type: "View", surface: fallback },
    );
    for (const [control, surface] of Object.entries(surfaces))
      assert.equal(
        surface,
        expected,
        `${liquidGlass ? "glass" : "fallback"} ${mode} ${control}`,
      );
  }
  // The call sites only lay out their capsules: none sets a surface key.
  for (const [file, block] of [
    [
      "components/workspace/WorkspaceQuickRail.tsx",
      /const styles = StyleSheet\.create\(\{[\s\S]*?\n\}\);/,
    ],
    [
      "features/assistant/ChatIdentityHeader.tsx",
      /floatingIdentity: \{[\s\S]*?floatingCapsule: \{[\s\S]*?\}/,
    ],
  ] as const) {
    const styles = read(file).match(block)![0];
    // The unread dot draws its own small badge; nothing else does.
    const withoutDot = styles.replace(/dot: \{[\s\S]*?\}/, "");
    assert.doesNotMatch(
      withoutDot,
      /backgroundColor|borderColor|borderWidth|shadow/,
      file,
    );
  }
});
