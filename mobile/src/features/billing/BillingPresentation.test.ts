import assert from "node:assert/strict";
import { URL, fileURLToPath } from "node:url";
import test from "node:test";
import { hookHarness } from "../assistant/assistantHookTestHarness";
import * as credits from "./credits";
import * as summary from "./balanceSummary";
import { allowancesSchema, grantsSchema, serviceUsage, usageSchema, walletSchema } from "./billingContracts";
import { allowanceFixture, grantFixture, usageFixture, walletFixture } from "./billingFixtures";

type Element = { type: unknown; props: Record<string, any> };
function elements(root: Element): Element[] {
  return [root, ...[root.props.children].flat(Infinity).flatMap((child) => child && typeof child === "object" && "props" in child ? elements(child) : [])];
}
const text = (root: Element) => elements(root).flatMap((element) => [element.props.children].flat(Infinity)).filter((value) => typeof value === "string").join(" ");

function setup(component: string, props: Record<string, unknown>, fontScale = 1) {
  const jsx = (type: unknown, props: Record<string, unknown>): Element => typeof type === "function" ? type(props) : { type, props };
  const harness = hookHarness<Element>({
    "react/jsx-runtime": { jsx, jsxs: jsx, Fragment: "Fragment" },
    "lucide-react-native": { ChevronDown: "ChevronDown", ChevronRight: "ChevronRight" },
    "react-native": { Pressable: "Pressable", View: "View", StyleSheet: { create: (styles: unknown) => styles }, useWindowDimensions: () => ({ fontScale }) },
    "../../components/workspace/WorkspaceUI": { ActionButton: "ActionButton", Card: "Card", Label: "Label" },
    "../../theme/ThemeContext": { useTheme: () => ({ colors: { danger: "red", textPrimary: "zinc", cardSoft: "zinc", border: "zinc" } }) },
    "../../theme/designTokens": { spacing: {}, radius: {}, typeScale: {} },
    "./credits": credits,
    "./balanceSummary": summary,
  });
  const components = harness.load(fileURLToPath(new URL("./BillingPresentation.tsx", import.meta.url))) as Record<string, (props: Record<string, unknown>) => Element>;
  harness.mount(() => components[component]!(props));
  return { harness, all: () => elements(harness.value()), text: () => text(harness.value()) };
}

test("figures expose exact tabular, right-aligned values and accessible labels", (t) => {
  const screen = setup("BillingFigure", { label: "Balance", value: "-0.000000000001" });
  t.after(screen.harness.unmount);
  assert.equal(screen.harness.value().props.accessibilityLabel, "Balance: -0.000000000001 credits");
  const amount = screen.all().find((item) => item.props.mono);
  assert.ok(amount);
  assert.equal(amount.props.style[0].textAlign, "right");
  assert.deepEqual(JSON.parse(JSON.stringify(amount.props.style[1].fontVariant)), ["tabular-nums"]);
  assert.equal(amount.props.style[2].color, "red");
  assert.equal(amount.props.numberOfLines, 1);
  assert.equal(amount.props.adjustsFontSizeToFit, true);
});

test("one balance card is large and exact, with only nonzero compact rows", (t) => {
  const screen = setup("BillingWalletCard", { wallet: walletSchema.parse(walletFixture), grants: [], allowances: [], now: Date.now() });
  t.after(screen.harness.unmount);
  const figure = screen.all().find((item) => item.props.accessibilityLabel === "Available credits: -1.334567890126")!;
  assert.ok(figure);
  assert.ok(figure.props.style[0].fontSize >= 28);
  assert.equal(figure.props.style[0].textAlign, "right");
  assert.deepEqual(JSON.parse(JSON.stringify(figure.props.style[0].fontVariant)), ["tabular-nums"]);
  assert.equal(figure.props.style[1].color, "red");
  assert.equal(figure.props.numberOfLines, 1);
  assert.equal(figure.props.adjustsFontSizeToFit, true);
  assert.ok(figure.props.minimumFontScale > 0);
  assert.ok(screen.text().includes("-1.334568"), "visible balance rounds to six decimals while VoiceOver retains exact credits");
  const labels = screen.all().map((item) => item.props.accessibilityLabel).filter(Boolean);
  assert.ok(labels.includes("Credit limit: 100 credits"));
  assert.ok(labels.includes("Pending / reserved: 0.100000000002 credits"));
  assert.ok(labels.includes("Pending expiry: 1.234567890123 credits"));
  assert.equal(screen.all().filter((item) => item.type === "Card").length, 1);
  assert.doesNotMatch(screen.text(), /Personal wallet|Updated|Available with overdraft|Active/);
});

test("spendable grants and allowances appear as compact rows within the balance card", (t) => {
  const screen = setup("BillingWalletCard", {
    wallet: walletSchema.parse(walletFixture), now: Date.parse("2026-10-28T00:00:00Z"),
    grants: grantsSchema.parse({ grants: [{ ...grantFixture, remaining: "10.000000000002", held: "0.000000000001" }] }).grants,
    allowances: allowancesSchema.parse({ allowances: [allowanceFixture] }).allowances,
  });
  t.after(screen.harness.unmount);
  assert.match(screen.text(), /Included credits · example · service limits apply/);
  assert.match(screen.text(), /Expiring/);
  assert.match(screen.text(), /2026/);
  assert.match(screen.text(), /690 \/ 1,000 requests left this month/);
  assert.equal(screen.all().filter((item) => item.type === "Card").length, 1);
  assert.doesNotMatch(screen.text(), /Development credits|Granted|Consumed/);
});

test("per-service history is one exact figure per service", (t) => {
  const screen = setup("BillingServiceCard", { service: serviceUsage(usageSchema.parse(usageFixture).rows)[0] });
  t.after(screen.harness.unmount);
  assert.ok(screen.all().some((item) => item.props.accessibilityLabel === "example: 9,007,199,254,740,993.000000000003 credits"));
  assert.equal(screen.all().find((item) => item.props.mono)!.props.children.join(""), "9,007,199,254,740,993 credits");
  assert.doesNotMatch(screen.text(), /From wallet|From grants|input tokens|Show more usage/);
});

test("large text stacks labels and fits each whole credit amount while preserving exact VoiceOver values", (t) => {
  const screen = setup("BillingFigure", { label: "Pending", value: "1.875", exactValue: "1.875000000002" }, 3.1);
  t.after(screen.harness.unmount);
  assert.equal(screen.harness.value().props.style[1].flexDirection, "column");
  assert.equal(screen.harness.value().props.accessibilityLabel, "Pending: 1.875000000002 credits");
  const amount = screen.all().find((item) => item.props.mono)!;
  assert.equal(amount.props.style[3].width, "100%");
  assert.equal(amount.props.style[3].textAlign, "left");
  assert.equal(amount.props.style[3].alignSelf, "flex-start");
  assert.equal(amount.props.numberOfLines, 1);
  assert.equal(amount.props.adjustsFontSizeToFit, true);
  assert.equal(amount.props.children.join(""), "1.875 credits");
});

test("allowance descriptions use the body font and wrap words at large text sizes", (t) => {
  const screen = setup("BillingFigure", { label: "Included allowance", value: "120 / 500 requests left this month", unit: "" }, 3.1);
  t.after(screen.harness.unmount);
  const amount = screen.all().find((item) => item.props.mono === false)!;
  assert.ok(amount);
  assert.equal(amount.props.style[1], false, "no tabular font variant on descriptive text");
  assert.equal(amount.props.numberOfLines, undefined);
  assert.equal(amount.props.adjustsFontSizeToFit, false);
  assert.equal(amount.props.style[3].textAlign, "left");
  assert.equal(amount.props.style[3].alignSelf, "flex-start");
  assert.equal(screen.harness.value().props.accessibilityLabel, "Included allowance: 120 / 500 requests left this month");
});

test("history disclosure exposes expansion state and a 48pt target with a chevron", (t) => {
  let presses = 0;
  const screen = setup("BillingHistoryRow", { expanded: false, onPress: () => { presses++; } });
  t.after(screen.harness.unmount);
  const row = screen.harness.value();
  assert.equal(row.props.accessibilityRole, "button");
  assert.equal(row.props.accessibilityLabel, "Usage history");
  assert.equal(row.props.accessibilityState.expanded, false);
  assert.ok(row.props.style({ pressed: false })[0].minHeight >= 44);
  assert.ok(screen.all().some((item) => item.type === "ChevronRight"));
  row.props.onPress();
  assert.equal(presses, 1);
});
