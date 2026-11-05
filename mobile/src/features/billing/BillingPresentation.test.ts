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

function setup(component: string, props: Record<string, unknown>) {
  const jsx = (type: unknown, props: Record<string, unknown>): Element => typeof type === "function" ? type(props) : { type, props };
  const harness = hookHarness<Element>({
    "react/jsx-runtime": { jsx, jsxs: jsx, Fragment: "Fragment" },
    // Rows keep their props so tests read label, value and spoken label directly.
    "../../components/settings/SettingsList": {
      SettingsGroup: (props: Record<string, unknown>) => ({ type: "SettingsGroup", props }),
      SettingsRow: (props: Record<string, unknown>) => ({ type: "SettingsRow", props }),
    },
    "./credits": credits,
    "./balanceSummary": summary,
  });
  const components = harness.load(fileURLToPath(new URL("./BillingPresentation.tsx", import.meta.url))) as Record<string, (props: Record<string, unknown>) => Element>;
  harness.mount(() => components[component]!(props));
  return { harness, all: () => elements(harness.value()), text: () => text(harness.value()) };
}

const rows = (screen: ReturnType<typeof setup>) => screen.all().filter((item) => item.type === "SettingsRow").map((item) => item.props);

test("the balance group shows rounded values, speaks exact amounts and marks negatives red", (t) => {
  const screen = setup("BillingWalletCard", { wallet: walletSchema.parse(walletFixture), grants: [], allowances: [], now: Date.now() });
  t.after(screen.harness.unmount);
  const group = screen.all().find((item) => item.type === "SettingsGroup")!;
  assert.equal(group.props.title, "Balance");
  const available = rows(screen).find((row) => row.label === "Available credits")!;
  assert.equal(available.value, "-1.334568 credits", "visible balance rounds to six decimals");
  assert.equal(available.accessibilityLabel, "Available credits: -1.334567890126", "VoiceOver keeps exact credits");
  assert.equal(available.valueTone, "danger");
  const spoken = rows(screen).map((row) => row.accessibilityLabel);
  assert.ok(spoken.includes("Credit limit: 100 credits"));
  assert.ok(spoken.includes("Pending / reserved: 0.100000000002 credits"));
  assert.ok(spoken.includes("Pending expiry: 1.234567890123 credits"));
  assert.doesNotMatch(screen.text(), /Personal wallet|Updated|Available with overdraft|Active/);
});

test("grants stay in Balance while allowances group under their service with one row per metric", (t) => {
  const screen = setup("BillingWalletCard", {
    wallet: walletSchema.parse(walletFixture), now: Date.parse("2026-10-28T00:00:00Z"),
    grants: grantsSchema.parse({ grants: [{ ...grantFixture, remaining: "10.000000000002", held: "0.000000000001" }] }).grants,
    allowances: allowancesSchema.parse({ allowances: [allowanceFixture] }).allowances,
  });
  t.after(screen.harness.unmount);
  const titles = screen.all().filter((item) => item.type === "SettingsGroup").map((item) => item.props.title);
  assert.deepEqual(titles, ["Balance", "Included · example · this month"]);
  assert.ok(rows(screen).some((row) => /Included credits · example · service limits apply/.test(row.label)));
  assert.ok(rows(screen).some((row) => /^Expiring .*2026/.test(row.label)));
  assert.deepEqual(
    JSON.parse(JSON.stringify(rows(screen).find((row) => row.label === "Requests"))),
    { label: "Requests", value: "690 of 1,000 left" },
  );
  assert.doesNotMatch(screen.text(), /Development credits|Granted|Consumed/);
});

test("per-service history is one row per service with an exact spoken amount", (t) => {
  const screen = setup("BillingServiceCard", { service: serviceUsage(usageSchema.parse(usageFixture).rows)[0] });
  t.after(screen.harness.unmount);
  const [row] = rows(screen);
  assert.equal(row!.label, "example");
  assert.equal(row!.value, "9,007,199,254,740,993 credits");
  assert.equal(row!.accessibilityLabel, "example: 9,007,199,254,740,993.000000000003 credits");
});

test("usage history is a disclosure row that reports its expansion state", (t) => {
  let presses = 0;
  const screen = setup("BillingHistoryRow", { expanded: false, onPress: () => { presses++; } });
  t.after(screen.harness.unmount);
  const [row] = rows(screen);
  assert.equal(row!.label, "Usage history");
  assert.equal(row!.accessibilityState.expanded, false);
  assert.equal(row!.value, undefined);
  row!.onPress();
  assert.equal(presses, 1);
});
