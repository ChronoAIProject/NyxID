import assert from "node:assert/strict";
import test from "node:test";
import { URL, fileURLToPath } from "node:url";
import { hookHarness } from "./assistantHookTestHarness";
import { messageTime } from "./messageTime";
type E = { type: any; props: Record<string, any> };
const nodes = (n: any): E[] =>
  n?.props ? [n, ...[n.props.children].flat(Infinity).flatMap(nodes)] : [];
function expand(n: any): any {
  if (!n?.props) return n;
  if (typeof n.type === "function") return expand(n.type(n.props));
  return {
    ...n,
    props: {
      ...n.props,
      children: [n.props.children].flat(Infinity).map(expand),
    },
  };
}
function setup(props: any) {
  const modules = { copy: {} };
  const alerts: any[] = [],
    copied: string[] = [];
  const h = hookHarness<E>({
    "react/jsx-runtime": {
      jsx: (type: unknown, props: unknown) => ({ type, props }),
      jsxs: (type: unknown, props: unknown) => ({ type, props }),
    },
    "react-native": {
      ...Object.fromEntries(
        ["View", "Text", "Pressable", "Modal", "ScrollView"].map((n) => [n, n]),
      ),
      StyleSheet: { create: (v: unknown) => v },
      Alert: { alert: (...args: any[]) => alerts.push(args) },
    },
    "lucide-react-native": {
      Copy: "Copy",
      Check: "Check",
      ThumbsUp: "Up",
      ThumbsDown: "Down",
    },
    "expo-clipboard": {
      setStringAsync: async (text: string) => {
        copied.push(text);
        return true;
      },
    },
    "../../theme/ThemeContext": {
      useTheme: () => ({
        colors: {
          textMuted: "tertiary",
          primaryTone: {},
          successTone: { text: "success" },
          dangerTone: { text: "error" },
        },
      }),
    },
    "../../components/workspace/WorkspaceUI": Object.fromEntries(
      ["ActionButton", "Card", "Field", "Label"].map((n) => [n, n]),
    ),
    "./AssistantCopyButton": modules.copy,
  });
  Object.assign(
    modules.copy,
    h.load(
      fileURLToPath(new URL("./AssistantCopyButton.tsx", import.meta.url)),
    ),
  );
  const C = h.load(
    fileURLToPath(new URL("./AssistantMessageFooter.tsx", import.meta.url)),
  ).AssistantMessageFooter as any;
  h.mount(() => expand(C(props)));
  return { h, copied, alerts, all: () => nodes(h.value()) };
}
test("private and crew reply footers contain only compact Copy and date, 8pt apart and below the body", async () => {
  for (const reply of ["Private reply", "Crew reply"]) {
    const x = setup({ text: reply, timestamp: "2026-10-27T01:00:00Z" });
    const row = x.h.value();
    assert.equal(row.props.style[0].flexDirection, "row");
    assert.equal(row.props.style[0].gap, 8);
    assert.equal(row.props.style[0].marginTop, 8);
    const all = x.all();
    const controls = all.filter((n) => n.type === "Pressable");
    assert.equal(controls.length, 2);
    assert.equal(controls[0]!.props.accessibilityLabel, "Copy message");
    assert.equal(controls[1]!.props.accessibilityRole, "text");
    assert.equal(
      all.some((n) => ["Up", "Down", "Modal", "Field"].includes(n.type)),
      false,
    );
    const copy = controls[0]!;
    const style = copy.props.style({ pressed: false })[0];
    assert.equal(style.width, 24);
    assert.equal(style.height, 24);
    assert.equal(
      style.width + copy.props.hitSlop.left + copy.props.hitSlop.right,
      44,
    );
    assert.equal(
      style.height + copy.props.hitSlop.top + copy.props.hitSlop.bottom,
      44,
    );
    assert.equal(
      copy.props.hitSlop.right,
      4,
      "copy expansion stops before the date",
    );
    assert.ok(
      all.some(
        (n) =>
          n.type === "Copy" &&
          n.props.size === 16 &&
          n.props.color === "tertiary",
      ),
    );
    const stamp = all.find((n) => n.type === "Text")!;
    assert.equal(stamp.props.style[0].fontSize, 11);
    assert.equal(stamp.props.style[1].color, "tertiary");
    assert.equal(controls[1]!.props.style.minWidth, undefined);
    copy.props.onPress();
    await x.h.settle();
    assert.deepEqual(x.copied, [reply]);
    assert.ok(x.all().some((n) => n.props.accessibilityLabel === "Copied"));
    controls[1]!.props.onLayout({
      nativeEvent: { layout: { width: 33, height: 15 } },
    });
    await x.h.settle();
    const date = x.all().filter((n) => n.type === "Pressable")[1]!;
    assert.equal(33 + date.props.hitSlop.left + date.props.hitSlop.right, 44);
    assert.equal(15 + date.props.hitSlop.top + date.props.hitSlop.bottom, 44);
    assert.equal(
      date.props.hitSlop.left,
      4,
      "date expansion meets copy at half the gap",
    );
    date.props.onLongPress();
    assert.match(x.alerts[0][1], /2026/);
    x.h.unmount();
  }
});
test("human footer keeps time-before-Copy order with the same compact spacing", () => {
  const x = setup({
    user: true,
    text: "Human",
    timestamp: "2026-10-27T01:00:00Z",
  });
  assert.equal(x.h.value().props.children[0].props.accessibilityRole, "text");
  assert.equal(
    x.h.value().props.children[1].props.accessibilityLabel,
    "Copy message",
  );
  assert.equal(x.h.value().props.style[0].gap, 8);
  assert.equal(x.h.value().props.children[1].props.hitSlop.left, 4);
  x.h.unmount();
});
test("user bubble dates and copy icons use the supplied foreground", () => {
  const x = setup({ text: "Hello", timestamp: "2026-10-27T01:00:00Z", user: true, foreground: "#FFFFFF" });
  const date = x.all().find((node) => node.type === "Text");
  assert.ok(date);
  assert.equal(Object.assign({}, ...[date.props.style].flat()).color, "#FFFFFF");
  assert.equal(x.all().find((node) => node.type === "Copy")?.props.color, "#FFFFFF");
  x.h.unmount();
});

test("message-time parity: local calendar day/month/year, locale and full daylight-saving time labels", () => {
  const now = Date.parse("2026-10-28T04:00:00Z");
  const short = (value: string, timeZone: string, locale = "en-US") =>
    messageTime(value, { locale, timeZone, now })?.short;
  assert.equal(short("2026-10-27T00:04:00Z", "Asia/Singapore"), "27 Oct");
  assert.equal(short("2025-10-27T00:04:00Z", "Asia/Singapore"), "27 Oct 2025");
  assert.equal(short("2026-12-31T16:00:00Z", "UTC"), "31 Dec");
  assert.equal(short("2026-12-31T16:00:00Z", "Asia/Tokyo"), "1 Jan 2027");
  assert.equal(
    short("2026-10-27T00:04:00Z", "Asia/Shanghai", "zh-CN"),
    "10月27日",
  );
  const options = { locale: "en-US", timeZone: "America/New_York", now };
  assert.match(
    messageTime("2026-11-01T05:30:00Z", options)!.label,
    /1:30 AM EDT/,
  );
  assert.match(
    messageTime("2026-11-01T06:30:00Z", options)!.label,
    /1:30 AM EST/,
  );
  assert.equal(
    messageTime(Date.parse("2026-10-27T00:04:00Z"), options)!.iso,
    "2026-10-27T00:04:00.000Z",
  );
  for (const invalid of [undefined, null, "", "invalid", NaN])
    assert.equal(messageTime(invalid), undefined);
});
