import assert from "node:assert/strict";
import test from "node:test";
import { fileURLToPath, URL } from "node:url";
import { lightColors } from "../../theme/mobileTheme";
import { hookHarness } from "./assistantHookTestHarness";

type Element = { type: unknown; props: Record<string, any> };
const nodes = (node: any): Element[] =>
  node?.props
    ? [node, ...[node.props.children].flat(Infinity).flatMap(nodes)]
    : [];
const flat = (style: unknown) =>
  Object.assign({}, ...[style].flat(Infinity).filter(Boolean));

function setup(platform = "ios", reducedMotion = false, sourceWidth = 88) {
  const actions: string[] = [];
  const focus: number[] = [];
  const animations: {
    config: any;
    complete: (result: { finished: boolean }) => void;
    stopped: boolean;
    value: any;
  }[] = [];
  let reduced = reducedMotion;
  let width = 390;
  class Value {
    value = 0;
    setValue(value: number) {
      this.value = value;
    }
    interpolate(config: unknown) {
      return { value: this, config };
    }
  }
  const props = {
    scope: "account:chat",
    disabled: false,
    options: ["details", "settings"].map((id) => ({
      id,
      label: id,
      Icon: "Icon",
      disabled: false,
      onPress: () => actions.push(id),
    })),
    controls: (open: () => void, trigger: any) => {
      if (trigger) trigger.current = { handle: 11 };
      return { type: "Trigger", props: { onPress: open } };
    },
  };
  const h = hookHarness<Element>({
    "react/jsx-runtime": {
      jsx: (type: unknown, props: unknown) => ({ type, props }),
      jsxs: (type: unknown, props: unknown) => ({ type, props }),
    },
    "react-native": {
      View: "View",
      Pressable: "Pressable",
      Modal: "Modal",
      ScrollView: "ScrollView",
      Text: "Text",
      StyleSheet: {
        create: (value: unknown) => value,
        hairlineWidth: 0.5,
        absoluteFill: {},
      },
      Platform: { OS: platform },
      useWindowDimensions: () => ({ width, height: 844, fontScale: 1 }),
      findNodeHandle: (view: { handle: number }) => view.handle,
      AccessibilityInfo: {
        setAccessibilityFocus: (handle: number) => focus.push(handle),
      },
      Easing: { out: (value: unknown) => value, cubic: "cubic" },
      Animated: {
        View: "Animated.View",
        Value,
        timing: (value: Value, config: unknown) => {
          const animation = {
            config,
            value,
            complete: (_result: { finished: boolean }) => {},
            stopped: false,
          };
          animations.push(animation);
          return {
            start: (callback: typeof animation.complete) => {
              animation.complete = callback;
            },
            stop: () => {
              animation.stopped = true;
            },
          };
        },
      },
    },
    "react-native-safe-area-context": {
      SafeAreaView: "SafeAreaView",
      useSafeAreaInsets: () => ({ top: 0, right: 0, left: 0, bottom: 0 }),
    },
    "../../components/FloatingControl": { FloatingCapsule: "Capsule" },
    "../../theme/ThemeContext": {
      useTheme: () => ({ colors: lightColors, mode: "light" }),
    },
    "../../lib/accessibility/useReducedMotion": {
      useReducedMotion: () => reduced,
    },
  });
  const Menu = h.load(
    fileURLToPath(new URL("./ChatHeaderMenu.tsx", import.meta.url)),
  ).ChatHeaderMenu as (props: any) => Element;
  h.mount(() => Menu(props));
  const all = () => nodes(h.value());
  const anchor = () =>
    all().find((node) => node.props.testID === "chat-header-actions")!;
  anchor().props.ref.current = {
    measureInWindow: (report: (...values: number[]) => void) =>
      report(width - 12 - sourceWidth, 52, sourceWidth, 44),
  };
  const modal = () => all().find((node) => node.type === "Modal")!;
  const menu = () =>
    all().find((node) => node.props.testID === "chat-header-menu")!;
  const open = () => {
    nodes(anchor())
      .find((node) => node.type === "Trigger")!
      .props.onPress();
    h.flush();
    all().find(
      (node) => node.props.accessibilityRole === "menuitem",
    )!.props.ref.current = { handle: 22 };
  };
  const complete = (index: number) => {
    const animation = animations[index]!;
    animation.value.setValue(animation.config.toValue);
    animation.complete({ finished: true });
    h.flush();
  };
  return {
    h,
    props,
    actions,
    focus,
    animations,
    all,
    anchor,
    modal,
    menu,
    open,
    complete,
    show: () => {
      modal().props.onShow();
      h.flush();
    },
    dismiss: () => {
      modal().props.onRequestClose();
      h.flush();
    },
    select: (id: string) => {
      all()
        .find((node) => node.props.accessibilityLabel === id)!
        .props.onPress();
      h.flush();
    },
    setReduced: (value: boolean) => {
      reduced = value;
      h.render();
    },
    setWidth: (value: number) => {
      width = value;
      h.render();
    },
  };
}

test("the Android menu positions its measured anchor inside the modal's native top safe area", () => {
  const x = setup("android");
  x.open();
  const surface = x.modal().props.children;
  assert.equal(surface.type, "SafeAreaView");
  assert.deepEqual(Array.from(surface.props.edges), ["top"]);
  assert.equal(surface.props.children.props.accessibilityViewIsModal, true);
  assert.equal(flat(x.menu().props.style).top, 52);
  x.h.unmount();
});

test("the source capsule stays until native presentation, then grows and shrinks at its exact anchor before dispatching", () => {
  const x = setup();
  x.open();
  assert.ok(
    x.anchor().props.children,
    "source remains visible before Modal onShow",
  );
  const style = flat(x.menu().props.style);
  assert.equal(style.top, 52);
  assert.equal(style.right, 12);
  assert.deepEqual(Array.from(style.width.config.outputRange), [88, 240]);
  assert.deepEqual(Array.from(style.height.config.outputRange), [44, 96]);
  x.show();
  assert.equal(x.anchor().props.children, null);
  assert.equal(x.animations[0]!.config.duration, 220);
  x.complete(0);
  assert.deepEqual(x.focus, [22]);
  x.select("details");
  assert.equal(
    x.modal().props.visible,
    true,
    "modal stays until reverse animation ends",
  );
  assert.deepEqual(x.actions, []);
  assert.equal(x.animations[1]!.config.toValue, 0);
  x.complete(1);
  assert.equal(x.modal().props.visible, false);
  assert.deepEqual(x.actions, [], "iOS waits for native dismissal too");
  x.modal().props.onDismiss();
  x.modal().props.onDismiss();
  assert.deepEqual(x.actions, ["details"]);
  x.h.unmount();
});

test("closing during opening cancels its focus callback, then restores the exact trigger once", () => {
  const x = setup();
  x.open();
  x.show();
  x.dismiss();
  assert.equal(x.animations[0]!.stopped, true);
  x.complete(0);
  assert.deepEqual(x.focus, [], "stale opening completion cannot steal focus");
  x.complete(1);
  x.modal().props.onDismiss();
  x.modal().props.onDismiss();
  assert.deepEqual(x.focus, [11]);
  assert.deepEqual(x.actions, []);
  x.h.unmount();
});

for (const interruption of [
  "blur",
  "scope",
  "options",
  "disabled-option",
  "resize",
]) {
  test(`${interruption} while closing cancels the selected destination`, () => {
    const x = setup();
    x.open();
    x.show();
    x.complete(0);
    x.select("details");
    const staleDismiss = x.modal().props.onDismiss;
    if (interruption === "blur") {
      x.props.disabled = true;
      x.h.render();
      x.props.disabled = false;
    }
    if (interruption === "scope") x.props.scope = "different-chat";
    if (interruption === "options") x.props.options = x.props.options.slice(1);
    if (interruption === "disabled-option") x.props.options[0]!.disabled = true;
    if (interruption === "resize") x.setWidth(600);
    x.h.render();
    x.complete(1);
    staleDismiss();
    assert.deepEqual(x.actions, []);
    x.h.unmount();
  });
}

for (const platform of ["ios", "android"]) {
  test(`${platform}: Reduce Motion skips size travel and keeps dismissal/action ordering`, () => {
    const x = setup(platform, true, 44);
    x.open();
    x.show();
    assert.deepEqual(
      Array.from(flat(x.menu().props.style).width.config.outputRange),
      [44, 240],
    );
    assert.equal(x.animations.length, 0);
    x.select("settings");
    if (platform === "ios") {
      assert.deepEqual(x.actions, []);
      x.modal().props.onDismiss();
    }
    assert.deepEqual(x.actions, ["settings"]);
    x.h.unmount();
  });
}

test("enabling Reduce Motion interrupts a running size animation", () => {
  const x = setup();
  x.open();
  x.show();
  x.setReduced(true);
  assert.equal(x.animations[0]!.stopped, true);
  assert.equal(flat(x.menu().props.style).width.value.value, 1);
  x.dismiss();
  x.modal().props.onDismiss();
  assert.equal(x.animations.length, 1);
  x.h.unmount();
});

test("light bubble has a rounded visible outline and theme elevation outside the clipped surface", () => {
  const x = setup();
  x.open();
  const outer = flat(x.menu().props.style);
  assert.equal(outer.borderRadius, 22);
  assert.equal(outer.borderWidth, 1);
  assert.equal(outer.borderColor, lightColors.border);
  assert.equal(outer.shadowColor, lightColors.shadowColor);
  assert.ok(outer.shadowRadius > 0 && outer.elevation > 0);
  assert.notEqual(outer.overflow, "hidden");
  const surface = nodes(x.menu()).find((node) => node.type === "Capsule")!;
  assert.equal(surface.props.style.overflow, "hidden");
  x.h.unmount();
});
