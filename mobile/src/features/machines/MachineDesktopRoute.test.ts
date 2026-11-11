import assert from "node:assert/strict";
import test from "node:test";
import { URL, fileURLToPath } from "node:url";
import { hookHarness } from "../assistant/assistantHookTestHarness";
import { desktopRouteParams } from "./desktopEntry";
import { createDesktopPrivacyQueue } from "./desktopPrivacy";

type Element = { type: unknown; props: Record<string, any> };
function elements(root: Element): Element[] {
  return [root, ...[root.props.children].flat(Infinity).flatMap((child) =>
    child && typeof child === "object" && "props" in child ? elements(child) : [],
  )];
}
function setup(privacyAvailable: boolean) {
  const calls: string[] = [];
  const jsx = (type: unknown, props: Record<string, unknown>) => ({ type, props });
  const harness = hookHarness<Element>({
    "react/jsx-runtime": { jsx, jsxs: jsx },
    "react-native": { Platform: { OS: "ios" } },
    "@react-navigation/native": { useIsFocused: () => true },
    "../../components/workspace/WorkspaceUI": {
      WorkspacePage: "Page", Notice: "Notice", Label: "Label", ActionButton: "Button",
    },
    "./desktop/MachineDesktopScreen": { MachineDesktopScreen: "NativeDesktop" },
    "./desktopEntry": { desktopRouteParams },
    "./desktopPrivacy": { desktopPrivacyQueue: createDesktopPrivacyQueue() },
    "expo-screen-capture": {
      preventScreenCaptureAsync: async () => {
        calls.push("protect");
        if (!privacyAvailable) throw new Error("Native module missing");
      },
      enableAppSwitcherProtectionAsync: async () => { calls.push("app-switcher"); },
      disableAppSwitcherProtectionAsync: async () => { calls.push("remove-app-switcher"); },
      allowScreenCaptureAsync: async () => { calls.push("unprotect"); },
    },
  });
  const Route = harness.load(fileURLToPath(new URL("./MachineDesktopRoute.tsx", import.meta.url))).MachineDesktopRoute as (props: any) => Element;
  harness.mount(() => Route({
    route: { params: { nodeId: "node", conversation_id: "thread", context_id: "context", display: "dev" } },
    navigation: { goBack: () => { calls.push("close"); } },
  }));
  return { harness, calls };
}

test("desktop entry opens only the native viewer with preserved selectors after privacy protection", async () => {
  const x = setup(true);
  const root = await x.harness.settle();
  assert.equal(root.type, "NativeDesktop");
  assert.deepEqual(JSON.parse(JSON.stringify(root.props.route.params)), {
    nodeId: "node", conversationId: "thread", contextId: "context", display: "dev",
  });
  assert.equal(root.props.privacyProtected, true);
  assert.deepEqual(x.calls, ["protect", "app-switcher"]);
  x.harness.unmount();
  await x.harness.settle();
  assert.deepEqual(x.calls.slice(-2), ["remove-app-switcher", "unprotect"]);
});

test("privacy failure stays native with an error and Close; no desktop web fallback", async () => {
  const x = setup(false);
  const root = await x.harness.settle();
  const all = elements(root);
  assert.equal(all.some((row) => row.type === "NativeDesktop"), false);
  assert.match(all.find((row) => row.type === "Notice")!.props.message, /privacy protection is unavailable/);
  const buttons = all.filter((row) => row.type === "Button");
  assert.deepEqual(buttons.map((row) => row.props.label), ["Close desktop"]);
  buttons[0]!.props.onPress();
  assert.equal(x.calls.at(-1), "close");
  x.harness.unmount();
  await x.harness.settle();
});
