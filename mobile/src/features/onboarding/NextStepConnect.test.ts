import assert from "node:assert/strict";
import test from "node:test";
import { URL, fileURLToPath } from "node:url";
import { hookHarness } from "../assistant/assistantHookTestHarness";
import type { SuggestionSetup } from "./onboardingContracts";
type E = { type: unknown; props: Record<string, any> };
const choices = [
  { slug: "api-google-calendar", name: "Google Calendar", scopes: [] },
  { slug: "api-microsoft", name: "Microsoft 365", scopes: ["Calendars.ReadWrite"] },
];
function nodes(value: any): E[] {
  if (!value || typeof value !== "object") return [];
  if (Array.isArray(value)) return value.flatMap(nodes);
  return [value, ...nodes(value.props?.children)];
}
const step = { id: "connect_calendar", title: "Connect your calendar", prompt: "Help me connect my calendar.", connect: true };
function fixture(initial: Partial<SuggestionSetup> = {}) {
  let setup: SuggestionSetup = { connected: false, choices, prompt: "Use my existing connected account. Plan my day.", ...initial };
  const minted: unknown[][] = [], opened: string[] = [], started: any[] = [];
  let closed = 0;
  const h = hookHarness<E>({
    "react/jsx-runtime": { jsx: (type: any, props: any) => ({ type, props }), jsxs: (type: any, props: any) => ({ type, props }) },
    "react-native": { View: "View", Linking: { openURL: async (url: string) => opened.push(url) } },
    "../../components/workspace/WorkspaceUI": { ActionButton: "Button", Label: "Label" },
    "../../lib/api/onboardingApi": { onboardingApi: { suggestionSetup: async () => setup } },
    "../../lib/api/connectLinksApi": { connectLinksApi: { create: async (...args: unknown[]) => { minted.push(args); return { connect_url: "https://nyxid.test/connect/nyx_clk_token" }; } } },
    "../assistant/AssistantHostedConnect": { AssistantHostedConnect: "HostedConnect" },
    "../assistant/assistantLinkTarget": { assistantLinkTarget: (url: string) => ({ kind: "connect", token: url.split("/").at(-1) }) },
    "../../lib/env": { firstPartyFrontendUrls: () => ["https://nyxid.test"] },
    "../groups/useGroupRuntime": { useGroupAvailability: () => ({ active: true }) },
  });
  const Component = h.load(fileURLToPath(new URL("./NextStepConnect.tsx", import.meta.url))).NextStepConnect as any;
  h.mount(() => Component({ step, onStart: (step: any) => started.push(step), onClose: () => { closed++; } }));
  return { h, minted, opened, started,
    closed: () => closed,
    change: (patch: Partial<SuggestionSetup>) => { setup = { ...setup, ...patch }; },
    buttons: () => nodes(h.value()).filter((n) => n.type === "Button"),
  };
}
for (const choice of choices) {
  test(`suggested calendar opens hosted ${choice.name}, then continues with the verified connection`, async () => {
    const f = fixture(); await f.h.settle();
    await f.buttons().find((n) => n.props.label === `Connect ${choice.name}`)!.props.onPress();
    await f.h.settle();
    assert.deepEqual(f.minted, [[choice.slug, undefined, choice.scopes]]);
    assert.deepEqual(f.opened, []);
    const hosted = nodes(f.h.value()).find((n) => n.type === "HostedConnect")!;
    assert.equal(hosted.props.token, "nyx_clk_token");
    assert.equal(f.started.length, 0);
    f.change({ connected: true, choices: [] });
    await hosted.props.onComplete();
    await f.h.settle();
    assert.equal(f.started.length, 1);
    assert.equal(f.started[0].connect, false);
    assert.match(f.started[0].prompt, /existing connected account/);
  });
}
test("suggested calendar omits unavailable providers", async () => {
  const f = fixture(); f.change({ choices: choices.slice(0, 1) });
  await f.h.settle();
  // Recheck the live snapshot on selection, including removal after the first read.
  await f.buttons().find((n) => n.props.label === "Connect Microsoft 365")!.props.onPress();
  await f.h.settle();
  assert.equal(f.minted.length, 0);
  assert.ok(!f.buttons().some((n) => n.props.label === "Connect Microsoft 365"));
});
test("an account connected before the click is reused without minting", async () => {
  const f = fixture(); await f.h.settle();
  f.change({ connected: true, choices: [] });
  await f.buttons().find((n) => n.props.label === "Connect Google Calendar")!.props.onPress();
  await f.h.settle();
  assert.equal(f.minted.length, 0);
  assert.equal(f.started.length, 1);
});

for (const when of ["first read", "selection recheck"]) {
  test(`no providers on ${when} starts the original prompt once without minting a link`, async () => {
    const f = fixture(when === "first read" ? { choices: [] } : {});
    await f.h.settle();
    if (when === "selection recheck") {
      f.change({ choices: [] });
      await f.buttons().find((n) => n.props.label === "Connect Google Calendar")!.props.onPress();
      await f.h.settle();
    }
    assert.deepEqual(f.started.map((value) => ({ ...value })), [{ ...step, connect: false }]);
    assert.equal(f.closed(), 1);
    assert.equal(f.minted.length, 0);
    assert.ok(!JSON.stringify(f.h.value()).includes("No supported provider"));
  });
}

test("no providers after hosted completion starts the original prompt only once", async () => {
  const f = fixture(); await f.h.settle();
  await f.buttons().find((n) => n.props.label === "Connect Google Calendar")!.props.onPress();
  await f.h.settle();
  const hosted = nodes(f.h.value()).find((n) => n.type === "HostedConnect")!;
  f.change({ choices: [] });
  await hosted.props.onComplete();
  await f.h.settle();
  // A late duplicate completion callback must not start another conversation.
  await hosted.props.onComplete();
  await f.h.settle();
  assert.deepEqual(f.started.map((value) => ({ ...value })), [{ ...step, connect: false }]);
  assert.equal(f.closed(), 1);
  assert.equal(f.minted.length, 1);
});


test("Home keeps the selected setup mounted when refreshed suggestions remove it", async () => {
  const step = { id: "connect_calendar", title: "Connect your calendar", connect: true };
  let steps: any[] = [step];
  const h = hookHarness<E>({
    "react/jsx-runtime": { jsx: (type: any, props: any) => ({ type, props }), jsxs: (type: any, props: any) => ({ type, props }) },
    "react-native": { View: "View", Pressable: "Pressable", StyleSheet: { create: (styles: any) => styles, hairlineWidth: 1 } },
    "@expo/vector-icons": { Feather: "Icon" },
    "../../components/workspace/WorkspaceUI": { Label: "Label" },
    "../../theme/ThemeContext": { useTheme: () => ({ colors: {} }) },
    "../../theme/designTokens": { fonts: {} },
    "./onboardingContracts": { featherIcon: () => "calendar" },
    "./NextStepConnect": { NextStepConnect: "Connect" },
  });
  const Component = h.load(fileURLToPath(new URL("./NextStepList.tsx", import.meta.url))).NextStepList as any;
  h.mount(() => Component({ steps, onStart: () => undefined }));
  nodes(h.value()).find((n) => n.type === "Pressable")!.props.onPress();
  await h.settle();
  steps = [];
  h.render();
  assert.equal(nodes(h.value()).find((n) => n.type === "Connect")?.props.step.id, step.id);
});
