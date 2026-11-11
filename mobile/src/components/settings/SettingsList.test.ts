import assert from "node:assert/strict";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { hookHarness } from "../../features/assistant/assistantHookTestHarness";
import {
  COMPOSER_ACCESS_DESCRIPTION,
  COMPOSER_ACCESS_LABEL,
} from "../../features/assistant/composerSettings";

type Element = { type: string; props: Record<string, any> };
const nodes = (node: any): Element[] =>
  node?.props
    ? [node, ...[node.props.children].flat(Infinity).flatMap(nodes)]
    : [];

function setup(
  y = 360,
  windowHeight = 873,
  options?: readonly { value: string; label: string; detail?: string }[],
) {
  let disabled = false;
  const changes: string[] = [];
  const dimensions = { width: 393, height: windowHeight, fontScale: 1 };
  const h = hookHarness<any>({
    "react/jsx-runtime": {
      jsx: (type: string, props: any) => ({ type, props }),
      jsxs: (type: string, props: any) => ({ type, props }),
    },
    "react-native": {
      ...Object.fromEntries(
        ["View", "Modal", "Pressable", "ScrollView"].map((name) => [name, name]),
      ),
      StyleSheet: {
        create: (styles: object) => styles,
        hairlineWidth: 0.5,
        absoluteFillObject: {},
      },
      useWindowDimensions: () => dimensions,
    },
    "lucide-react-native": { Check: "Check" },
    "../DynamicTypeText": { DynamicTypeText: "Text" },
    "../ScreenContainer": { ScreenContainer: "ScreenContainer" },
    "../workspace/ScrollEdgeFades": { PageScrollView: "PageScrollView" },
    "../../theme/ThemeContext": { useTheme: () => ({ colors: {} }) },
  });
  const { usePopupMenu } = h.load(
    fileURLToPath(new URL("./SettingsList.tsx", import.meta.url)),
  ) as any;
  h.mount(() =>
    usePopupMenu({
      options: options ?? (["autopilot", "nyxmode", "ask_every_time"] as const).map(
        (value) => ({
          value,
          label: COMPOSER_ACCESS_LABEL[value],
          detail: COMPOSER_ACCESS_DESCRIPTION[value],
        }),
      ),
      value: "nyxmode",
      disabled,
      onChange: (value: string) => changes.push(value),
    }),
  );
  h.value().anchor.current = {
    measureInWindow: (callback: Function) => callback(16, y, 361, 50),
  };
  const all = () => nodes(h.value().popup);
  const surface = () =>
    all().find((node) => node.props.accessibilityRole === "menu")!;
  const position = () => Object.assign({}, ...surface().props.style);
  const scroll = () => all().find((node) => node.type === "ScrollView")!;
  return {
    h,
    all,
    changes,
    position,
    dimensions,
    open: () => {
      h.value().open();
      h.flush();
    },
    measure: (height: number) => {
      scroll().props.onContentSizeChange(240, height);
      h.flush();
    },
    scroll,
    disable: () => {
      disabled = true;
      h.render();
    },
  };
}

test("long permission descriptions use available space so all three choices fit", () => {
  const x = setup();
  x.open();
  x.measure(330);
  assert.equal(x.position().top, 416);
  assert.ok(x.position().maxHeight >= 330);
  assert.equal(x.position().height, 331);
  assert.equal(x.scroll().props.showsVerticalScrollIndicator, true);
  const items = x.all().filter((node) => node.props.accessibilityRole === "menuitem");
  assert.equal(items.length, 3);
  assert.ok(items[2].props.accessibilityLabel.startsWith("Ask every time."));
  items[2].props.onPress();
  x.h.flush();
  assert.deepEqual(x.changes, ["ask_every_time"]);
  assert.equal(x.h.value().isOpen, false);
});

test("measured wrapped content flips above the trigger and stays bottom anchored", () => {
  const x = setup(440);
  x.open();
  assert.equal(x.position().top, 496);
  x.measure(400);
  assert.equal(x.position().top, undefined);
  assert.equal(x.position().bottom, 439);
  assert.equal(x.position().maxHeight, 378);
  // Font-size changes can increase native content height again without moving
  // the lower edge onto the trigger or losing access to the scrollable choices.
  x.measure(600);
  assert.equal(x.position().bottom, 439);
  assert.equal(x.position().maxHeight, 378);
  assert.equal(x.scroll().props.showsVerticalScrollIndicator, true);
});

test("small windows cap scrolling at actual safe space instead of a fixed minimum", () => {
  const x = setup(112, 300);
  x.open();
  x.measure(500);
  assert.equal(x.position().top, 168);
  assert.equal(x.position().maxHeight, 76);
  assert.equal(x.position().height, 76);
  assert.ok(x.position().top + x.position().maxHeight <= 244);
});

test("short menus shrink to measured content and later text wrapping updates their height", () => {
  const x = setup(360, 873, [{ value: "nyxmode", label: "Auto" }]);
  x.open();
  x.measure(48);
  assert.equal(x.position().height, 49);
  x.measure(120);
  assert.equal(x.position().height, 121);
  assert.ok(x.position().height < x.position().maxHeight);
});

test("disabling an open menu dismisses it and ignores late content measurements", () => {
  const x = setup();
  x.open();
  const onContentSizeChange = x.scroll().props.onContentSizeChange;
  x.disable();
  assert.equal(x.h.value().isOpen, false);
  onContentSizeChange(240, 400);
  x.h.flush();
  assert.equal(x.h.value().isOpen, false);
  assert.deepEqual(x.changes, []);
});

test("window resizing dismisses stale anchored menus and allows opening them again", () => {
  for (const resized of [
    { width: 873, height: 393 },
    { width: 320, height: 873 },
  ]) {
    const x = setup(440);
    x.open();
    x.measure(400);
    const onContentSizeChange = x.scroll().props.onContentSizeChange;
    // Ordinary re-renders at the initial size must retain the open menu.
    x.h.render();
    assert.equal(x.h.value().isOpen, true);
    Object.assign(x.dimensions, resized);
    x.h.render();
    assert.equal(x.h.value().isOpen, false);
    onContentSizeChange(240, 600);
    x.h.flush();
    assert.equal(x.h.value().isOpen, false);
    x.open();
    assert.equal(x.h.value().isOpen, true);
    assert.deepEqual(x.changes, []);
  }
});

test("a delayed anchor measurement cannot reopen a menu after the window changes", () => {
  const x = setup(440);
  const pending: Function[] = [];
  x.h.value().anchor.current = {
    measureInWindow: (callback: Function) => pending.push(callback),
  };
  x.open();
  assert.equal(x.h.value().isOpen, false);
  Object.assign(x.dimensions, { width: 873, height: 393 });
  x.h.render();
  pending.shift()!(16, 440, 361, 50);
  x.h.flush();
  assert.equal(x.h.value().isOpen, false);
  // A new request uses coordinates measured in the current window.
  x.open();
  pending.shift()!(16, 80, 361, 50);
  x.h.flush();
  assert.equal(x.h.value().isOpen, true);
  assert.equal(x.position().top, 136);
});
