import assert from "node:assert/strict";
import test from "node:test";
import { URL, fileURLToPath } from "node:url";
import { hookHarness } from "../assistant/assistantHookTestHarness";
import { darkColors, lightColors } from "../../theme/mobileTheme";

type E = { type: unknown; props: Record<string, any> };
const nodes = (n: any): E[] =>
  n?.props ? [n, ...[n.props.children].flat(Infinity).flatMap(nodes)] : [];

function render(props: Record<string, unknown>, colors = lightColors) {
  const h = hookHarness<E>({
    "react/jsx-runtime": {
      jsx: (type: unknown, props: unknown) => ({ type, props }),
      jsxs: (type: unknown, props: unknown) => ({ type, props }),
    },
    "react-native": {
      Pressable: "Pressable",
      View: "View",
      StyleSheet: { create: (v: unknown) => v },
    },
    "lucide-react-native": { Bell: "Bell" },
    "../../components/DynamicTypeText": { DynamicTypeText: "Text" },
    "../../theme/ThemeContext": { useTheme: () => ({ colors }) },
  });
  const { NotificationBell } = h.load(
    fileURLToPath(new URL("./NotificationBell.tsx", import.meta.url)),
  ) as { NotificationBell: (props: object) => E };
  const root = h.mount(() => NotificationBell(props));
  const pressable: E = { ...root, props: { ...root.props, style: root.props.style({ pressed: false }) } };
  return { h, root: pressable, all: () => nodes(pressable) };
}

test("the bell names its counts, caps the badge and uses one amber-capable indicator", () => {
  const pressed: number[] = [];
  const x = render({ unread: 23, needsDecision: 2, onPress: () => pressed.push(1) });
  assert.equal(x.root.props.accessibilityLabel, "Notifications, 23 unread, 2 need your decision");
  const badge = x.all().find((n) => n.props.testID === "notification-bell-badge")!;
  assert.equal(badge.props.style[1].backgroundColor, lightColors.warning);
  assert.equal(x.all().some((n) => n.props.testID === "notification-bell-attention"), false);
  assert.equal(nodes(badge).find((n) => n.type === "Text")!.props.children, "23");
  assert.ok(x.root.props.style[0].minWidth >= 44 && x.root.props.style[0].minHeight >= 44);
  x.root.props.onPress();
  assert.deepEqual(pressed, [1]);
  x.h.unmount();
  const quiet = render({ unread: 3, needsDecision: 0, onPress: () => {} });
  const plain = quiet.all().find((n) => n.props.testID === "notification-bell-badge")!;
  assert.equal(plain.props.style[1].backgroundColor, lightColors.primary);
  assert.equal(nodes(plain).find((n) => n.type === "Text")!.props.children, "3");
  quiet.h.unmount();
  const empty = render({ unread: 0, needsDecision: 0, onPress: () => {}, disabled: true });
  assert.equal(empty.root.props.accessibilityLabel, "Notifications");
  assert.equal(empty.all().some((n) => n.props.testID === "notification-bell-badge"), false);
  assert.equal(empty.root.props.disabled, true);
  empty.h.unmount();
});


for (const [mode, colors] of [["light", lightColors], ["dark", darkColors]] as const) {
  test(`one warning pill uses the ${mode} theme tone even at zero unread`, () => {
    const x = render({ unread: 0, needsDecision: 100, onPress: () => {} }, colors);
    const badge = x.all().find((n) => n.props.testID === "notification-bell-badge")!;
    assert.equal(badge.props.style[1].backgroundColor, colors.warning);
    const label = nodes(badge).find((n) => n.type === "Text")!;
    assert.equal(label.props.style[1].color, colors.onWarning);
    assert.equal(label.props.children, "99+");
    assert.equal(x.all().some((n) => n.props.testID === "notification-bell-attention"), false);
    x.h.unmount();
  });
}
