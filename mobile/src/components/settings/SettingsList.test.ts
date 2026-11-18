import assert from "node:assert/strict";
import { URL, fileURLToPath } from "node:url";
import test from "node:test";
import { hookHarness } from "../../features/assistant/assistantHookTestHarness";

type Element = { type: unknown; props: Record<string, any> };

function elements(root: Element): Element[] {
  return [
    root,
    ...[root.props.children]
      .flat(Infinity)
      .flatMap((child) =>
        child && typeof child === "object" && "props" in child
          ? elements(child)
          : [],
      ),
  ];
}

function setup() {
  let dimensions = { width: 1024, height: 1366, fontScale: 1 };
  const jsx = (type: unknown, props: Record<string, unknown>): Element => ({
    type,
    props,
  });
  const harness = hookHarness<any>({
    "react/jsx-runtime": { jsx, jsxs: jsx },
    "react-native": {
      ...Object.fromEntries(
        [
          "ActivityIndicator",
          "Modal",
          "Pressable",
          "RefreshControl",
          "ScrollView",
          "TextInput",
          "View",
        ].map((name) => [name, name]),
      ),
      useWindowDimensions: () => dimensions,
      StyleSheet: { create: <T>(styles: T) => styles, hairlineWidth: 1 },
    },
    "lucide-react-native": {},
    "../DynamicTypeText": { DynamicTypeText: "Text" },
    "../ScreenContainer": { ScreenContainer: "ScreenContainer" },
    "../workspace/ScrollEdgeFades": { PageScrollView: "PageScrollView" },
    "../../theme/ThemeContext": {
      useTheme: () => ({ colors: { bg: "background" } }),
    },
  });
  const components = harness.load(
    fileURLToPath(new URL("./SettingsList.tsx", import.meta.url)),
  ) as {
    SettingsPage: (props: any) => Element;
    SettingsRow: (props: any) => Element;
    usePopupMenu: (props: any) => {
      anchor: { current: unknown };
      open: () => void;
      popup: Element;
      isOpen: boolean;
    };
  };
  return {
    harness,
    ...components,
    resize: (width: number, height = 1366, fontScale = 1) => {
      dimensions = { width, height, fontScale };
      harness.render();
    },
  };
}

test("Settings rows use their measured card width when a large iPad window contains a narrow sheet", () => {
  for (const interactive of [false, true]) {
    const env = setup();
    env.harness.mount(() =>
      env.SettingsRow({
        label: "Long account setting label",
        value: "A value that must remain readable",
        accessory: "Control",
        onPress: interactive ? () => {} : undefined,
      }),
    );
    const value = () =>
      elements(env.harness.value()).find(
        (node) =>
          node.type === "Text" &&
          node.props.children === "A value that must remain readable",
      )!;
    assert.equal(value().props.style[1].textAlign, "right");
    env.harness
      .value()
      .props.onLayout({ nativeEvent: { layout: { width: 300 } } });
    env.harness.flush();
    assert.equal(value().props.style[1].textAlign, "left");
    assert.equal(env.harness.value().type, interactive ? "Pressable" : "View");
    env.harness
      .value()
      .props.onLayout({ nativeEvent: { layout: { width: 680 } } });
    env.harness.flush();
    assert.equal(value().props.style[1].textAlign, "right");
    env.resize(1024, 1366, 2.1);
    assert.equal(
      value().props.style[1].textAlign,
      "left",
      "live Dynamic Type uses the retained row measurement",
    );
    env.harness.unmount();
  }
});

test("Settings pages center a readable column while retaining a full-width scrolling viewport", () => {
  const env = setup();
  const child = { type: "SettingsGroup", props: {} };
  env.harness.mount(() => env.SettingsPage({ children: child }));
  const scroller = env.harness.value().props.children;
  assert.equal(scroller.type, "PageScrollView");
  assert.equal(scroller.props.style.flex, 1);
  assert.equal(scroller.props.contentContainerStyle.width, "100%");
  assert.equal(scroller.props.contentContainerStyle.maxWidth, 720);
  assert.equal(scroller.props.contentContainerStyle.alignSelf, "center");
  env.resize(375, 1024);
  assert.equal(env.harness.value().props.children.props.children, child);
  env.harness.unmount();
});

test("resizing or rotating an iPad dismisses a popup before its old anchor can point outside the live window", () => {
  const env = setup();
  const chosen: string[] = [];
  env.harness.mount(() =>
    env.usePopupMenu({
      value: "system",
      options: [
        { value: "system", label: "System" },
        { value: "dark", label: "Dark" },
      ],
      onChange: (value: string) => chosen.push(value),
    }),
  );
  env.harness.value().open();
  env.harness.flush();
  assert.equal(env.harness.value().isOpen, true);
  env.resize(507, 1024);
  assert.equal(env.harness.value().isOpen, false);
  assert.deepEqual(chosen, []);
  env.harness.value().open();
  env.harness.flush();
  assert.equal(
    env.harness.value().isOpen,
    true,
    "the resized window can open a newly placed menu",
  );
  env.resize(1024, 507);
  assert.equal(env.harness.value().isOpen, false);
  env.harness.unmount();
});

test("a delayed anchor measurement cannot reopen a resized popup or overwrite a newer placement", () => {
  const env = setup();
  const measurements: ((
    x: number,
    y: number,
    width: number,
    height: number,
  ) => void)[] = [];
  env.harness.mount(() =>
    env.usePopupMenu({
      value: "system",
      options: [{ value: "system", label: "System" }],
      onChange() {},
    }),
  );
  env.harness.value().anchor.current = {
    measureInWindow: (callback: (typeof measurements)[number]) =>
      measurements.push(callback),
  };
  env.harness.value().open();
  env.resize(507, 1024);
  env.harness.value().open();
  measurements[0]!(800, 1200, 160, 52);
  env.harness.flush();
  assert.equal(
    env.harness.value().isOpen,
    false,
    "old wide-window coordinates never reopen the menu",
  );
  measurements[1]!(16, 200, 300, 52);
  env.harness.flush();
  assert.equal(env.harness.value().isOpen, true);
  const menu = elements(env.harness.value().popup).find(
    (node) => node.props.accessibilityRole === "menu",
  )!;
  assert.equal(menu.props.style[1].top, 258);
  measurements[0]!(800, 1200, 160, 52);
  env.harness.flush();
  assert.equal(
    elements(env.harness.value().popup).find(
      (node) => node.props.accessibilityRole === "menu",
    )!.props.style[1].top,
    258,
  );
  env.harness.unmount();
});

test("pending popup measurements lose authority after Close, disabling, or unmount", () => {
  for (const reason of ["close", "disabled", "unmount"] as const) {
    const env = setup();
    let disabled = false;
    let renders = 0;
    let measured!: (
      x: number,
      y: number,
      width: number,
      height: number,
    ) => void;
    env.harness.mount(() => {
      renders++;
      return env.usePopupMenu({
        disabled,
        value: "system",
        options: [{ value: "system", label: "System" }],
        onChange() {},
      });
    });
    env.harness.value().anchor.current = {
      measureInWindow: (callback: typeof measured) => {
        measured = callback;
      },
    };
    env.harness.value().open();
    if (reason === "close") {
      env.harness.value().popup.props.onRequestClose();
      env.harness.flush();
    } else if (reason === "disabled") {
      disabled = true;
      env.harness.render();
    } else {
      env.harness.unmount();
    }
    const priorRenders = renders;
    measured(800, 1200, 160, 52);
    env.harness.flush();
    assert.equal(env.harness.value().isOpen, false, reason);
    assert.equal(
      renders,
      priorRenders,
      `${reason} rejects the callback before setting state`,
    );
    env.harness.unmount();
  }
});
