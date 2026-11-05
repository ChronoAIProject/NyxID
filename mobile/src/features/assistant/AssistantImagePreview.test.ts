import assert from "node:assert/strict";
import test from "node:test";
import { URL, fileURLToPath } from "node:url";
import { hookHarness } from "./assistantHookTestHarness";

type Element = { type: unknown; props: Record<string, any> };
const nodes = (n: any): Element[] =>
  n?.props ? [n, ...[n.props.children].flat(Infinity).flatMap(nodes)] : [];

function setup() {
  let dismissed = 0;
  let errors = 0;
  let gesture: any;
  const props = {
    uri: "file:///private/preview.png" as string | undefined,
    label: "Photo",
    closeLabel: "Close attachment preview",
    onClose: () => dismissed++,
    onError: () => errors++,
  };
  const h = hookHarness<Element>({
    "react/jsx-runtime": {
      jsx: (type: unknown, props: any) => ({ type, props }),
      jsxs: (type: unknown, props: any) => ({ type, props }),
    },
    "react-native": {
      Modal: "Modal",
      View: "View",
      Image: "Image",
      Pressable: "Pressable",
      StyleSheet: { create: (s: unknown) => s },
      PanResponder: {
        create: (config: unknown) => {
          gesture = config;
          return { panHandlers: { gestureOwner: true } };
        },
      },
    },
    "react-native-safe-area-context": {
      SafeAreaProvider: "SafeAreaProvider",
      SafeAreaView: "SafeAreaView",
    },
    "lucide-react-native": { X: "X" },
    "../../components/workspace/WorkspaceUI": { Label: "Label" },
    "../../theme/ThemeContext": { useTheme: () => ({ colors: {} }) },
  });
  const C = h.load(
    fileURLToPath(new URL("./AssistantImagePreview.tsx", import.meta.url)),
  ).AssistantImagePreview as (props: any) => Element;
  h.mount(() => C(props));
  const all = () => nodes(h.value());
  const find = (type: string) => all().find((n) => n.type === type)!;
  return {
    h,
    props,
    all,
    find,
    gesture: () => gesture,
    dismissed: () => dismissed,
    errors: () => errors,
  };
}

test("full-screen image close control uses its own native safe area and a 44pt target outside the image", () => {
  const x = setup();
  const modal = x.find("Modal");
  assert.equal(modal.props.visible, true);
  assert.equal(modal.props.presentationStyle, "fullScreen");
  const provider = x.find("SafeAreaProvider");
  assert.equal(modal.props.children, provider);
  const safe = x.find("SafeAreaView");
  assert.equal(provider.props.children, safe);
  const header = [safe.props.children].flat()[0];
  const close = nodes(header).find(
    (n) => n.props.accessibilityLabel === x.props.closeLabel,
  )!;
  assert.ok(close);
  assert.ok(close.props.style.width >= 44 && close.props.style.height >= 44);
  assert.equal(
    nodes(header).some((n) => n.type === "Image"),
    false,
  );
  assert.equal(x.find("Image").props.source.uri, x.props.uri);
  x.props.uri = undefined;
  x.h.render();
  assert.equal(x.find("Modal").props.visible, false);
  assert.equal(
    x.all().some((n) => n.type === "Image"),
    false,
  );
  x.h.unmount();
});

test("close button, preview background, hardware Back and accessibility Escape share a local dismissal", () => {
  for (const action of ["button", "background", "back", "escape"]) {
    const x = setup();
    if (action === "button")
      x.all()
        .find((n) => n.props.accessibilityLabel === x.props.closeLabel)!
        .props.onPress();
    if (action === "background")
      x.all()
        .find((n) => n.props.accessibilityLabel === "Dismiss image preview")!
        .props.onPress();
    if (action === "back") x.find("Modal").props.onRequestClose();
    if (action === "escape")
      x.find("SafeAreaView").props.onAccessibilityEscape();
    assert.equal(x.dismissed(), 1, action);
    assert.equal(x.errors(), 0);
    x.h.unmount();
  }
});

test("vertical downward gestures dismiss, while short, upward and horizontal gestures do not", () => {
  const x = setup();
  assert.equal(x.all().find((n) => n.props.gestureOwner)?.type, "View");
  const pan = x.gesture();
  for (const g of [
    { dx: 0, dy: -100, vy: -1 },
    { dx: 100, dy: 20, vy: 0 },
    { dx: 0, dy: 5, vy: 1 },
  ])
    assert.equal(pan.onMoveShouldSetPanResponder({}, g), false);
  assert.equal(pan.onMoveShouldSetPanResponder({}, { dx: 5, dy: 30 }), true);
  pan.onPanResponderRelease({}, { dx: 5, dy: 30, vy: 0 });
  assert.equal(x.dismissed(), 0);
  pan.onPanResponderRelease({}, { dx: 5, dy: 81, vy: 0 });
  pan.onPanResponderRelease({}, { dx: 5, dy: 20, vy: 0.9 });
  assert.equal(x.dismissed(), 2);
  x.find("Image").props.onError();
  assert.equal(x.errors(), 1);
  x.h.unmount();
});
