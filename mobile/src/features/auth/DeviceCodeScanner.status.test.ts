import assert from "node:assert/strict";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { deferred, hookHarness } from "../assistant/assistantHookTestHarness";
import * as activity from "../../lib/systemStatusActivity";

type Element = { type: unknown; props: Record<string, any> };
function elements(node: Element): Element[] {
  return [node, ...[node.props.children].flat(Infinity).flatMap((child) =>
    child && typeof child === "object" && "props" in child ? elements(child) : [],
  )];
}

function setup(granted: boolean, enabledFeatures: string[] = [], options: {
  getAccountProfile?: () => Promise<unknown>;
} = {}) {
  const routed: string[] = [];
  let profileReads = 0;
  let sessionGeneration = 0;
  let onAppStateChange: ((state: string) => void) | undefined;
  let permission = { granted, canAskAgain: true };
  const jsx = (type: unknown, props: Element["props"]) => ({ type, props });
  const h = hookHarness<Element>({
    "../../lib/api/mobileApi": { mobileApi: { getAccountProfile: async () => {
      profileReads++;
      return options.getAccountProfile
        ? options.getAccountProfile()
        : { capabilities: { enabled_features: enabledFeatures } };
    } } },
    "../../lib/auth/sessionStore": { authSessionGeneration: () => sessionGeneration },
    "react/jsx-runtime": { jsx, jsxs: jsx },
    "react-native": {
      Platform: { OS: "ios" }, View: "View", Text: "Text", Pressable: "Pressable", ActivityIndicator: "ActivityIndicator",
      StyleSheet: { create: (styles: unknown) => styles, absoluteFill: {} },
      useWindowDimensions: () => ({ width: 466, height: 678 }),
      AppState: { addEventListener: (_event: string, listener: (state: string) => void) => {
        onAppStateChange = listener;
        return { remove() { onAppStateChange = undefined; } };
      } },
    },
    "react-native-safe-area-context": { useSafeAreaInsets: () => ({ top: 0, bottom: 34 }) },
    "expo-camera": { CameraView: "CameraView", useCameraPermissions: () => [permission, async () => {}, async () => {}] },
    "expo-status-bar": { StatusBar: "StatusBar" },
    "lucide-react-native": { Camera: "Camera", ChevronLeft: "ChevronLeft" },
    "./AuthSubmitButton": { AuthSubmitButton: "AuthSubmitButton" },
    "../../theme/ThemeContext": { useTheme: () => ({ colors: {}, mode: "light" }) },
    "../../lib/systemStatusActivity": activity,
  });
  const module = h.load(fileURLToPath(new URL("./DeviceCodeScanner.tsx", import.meta.url))) as typeof import("./DeviceCodeScanner");
  h.mount(() => module.DeviceCodeScanner({ onCancel() {}, onCode(code, kind) { routed.push(kind + "/" + code); }, onManualEntry() {} }));
  return {
    h, routed,
    camera: () => elements(h.value()).find((node) => node.type === "CameraView"),
    error: () => elements(h.value()).find((node) => node.props.accessibilityRole === "alert")?.props.children,
    profileReads: () => profileReads,
    changeSession: () => { sessionGeneration++; },
    appState: (state: string) => onAppStateChange?.(state),
    permission: (granted: boolean) => { permission = { ...permission, granted }; h.render(); },
  };
}

test("opening QR or granting permission does not move the rail until the native camera-ready event", () => {
  const x = setup(false);
  assert.equal(x.camera(), undefined);
  assert.equal(activity.systemStatusActivityActive(), false);
  x.permission(true);
  assert.ok(x.camera());
  assert.equal(activity.systemStatusActivityActive(), false, "mounted camera is not yet a capture event");
  x.camera()!.props.onCameraReady();
  x.h.render();
  assert.equal(activity.systemStatusActivityActive(), true);
  x.h.unmount();
  assert.equal(activity.systemStatusActivityActive(), false);
});

test("camera errors or revoked permission release activity without waiting for a UI timer", () => {
  const x = setup(true);
  x.camera()!.props.onCameraReady();
  x.h.render();
  assert.equal(activity.systemStatusActivityActive(), true);
  x.camera()!.props.onMountError();
  x.h.render();
  assert.equal(activity.systemStatusActivityActive(), false);
  x.camera()!.props.onCameraReady();
  x.h.render();
  x.permission(false);
  assert.equal(activity.systemStatusActivityActive(), false);
  x.permission(true);
  assert.equal(activity.systemStatusActivityActive(), false, "a replacement camera must become ready before claiming activity");
  x.camera()!.props.onCameraReady();
  x.h.render();
  assert.equal(activity.systemStatusActivityActive(), true);
  x.h.unmount();
  assert.equal(activity.systemStatusActivityActive(), false);
});

test("only a flagged iOS account can leave the scanner with a staging request", async () => {
  for (const enabled of [false, true]) {
    const screen = setup(true, enabled ? ["auth:staging-qr-login"] : []);
    await screen.camera()!.props.onBarcodeScanned({ data: "https://staging.nyxid.ai/login/device?user_code=ABCD-EFGH" });
    await screen.h.settle();
    assert.deepEqual(screen.routed, enabled ? ["staging-device/ABCDEFGH"] : []);
    screen.h.unmount();
  }
});

test("a valid staging QR without account access explains the restriction", async (t) => {
  const screen = setup(true);
  t.after(() => screen.h.unmount());
  await screen.camera()!.props.onBarcodeScanned({ data: "https://staging.nyxid.ai/login/device?user_code=ABCD-EFGH" });
  await screen.h.settle();
  assert.deepEqual(screen.routed, []);
  assert.equal(screen.profileReads(), 1);
  assert.equal(screen.error(), "Staging sign-in is not enabled for this account. Ask your administrator for access.");
});

test("a failed staging account check offers retry instead of claiming the QR is invalid", async (t) => {
  const screen = setup(true, [], { getAccountProfile: async () => { throw new Error("offline"); } });
  t.after(() => screen.h.unmount());
  await screen.camera()!.props.onBarcodeScanned({ data: "https://staging-api.nyxid.ai/login/device?user_code=ABCD-EFGH" });
  await screen.h.settle();
  assert.deepEqual(screen.routed, []);
  assert.equal(screen.error(), "Could not check your staging sign-in access. Try scanning again.");
});

test("ordinary production QR scanning does not require a staging account check", async (t) => {
  const screen = setup(true, [], { getAccountProfile: async () => { throw new Error("offline"); } });
  t.after(() => screen.h.unmount());
  await screen.camera()!.props.onBarcodeScanned({ data: "nyxid://login/device?user_code=ABCD-EFGH" });
  await screen.h.settle();
  assert.deepEqual(screen.routed, ["device/ABCDEFGH"]);
  assert.equal(screen.profileReads(), 0);
  assert.equal(screen.error(), undefined);
});

test("untrusted QR origins stay invalid without reading account access", async (t) => {
  const screen = setup(true, ["auth:staging-qr-login"]);
  t.after(() => screen.h.unmount());
  await screen.camera()!.props.onBarcodeScanned({ data: "https://staging.nyxid.ai.evil.test/login/device?user_code=ABCD-EFGH" });
  await screen.h.settle();
  assert.deepEqual(screen.routed, []);
  assert.equal(screen.profileReads(), 0);
  assert.equal(screen.error(), "This QR code cannot be used to sign in to NyxID. Scan the sign-in QR code shown on the NyxID website or in your terminal.");
});

for (const invalidate of ["session", "background", "unmount"] as const) {
  for (const outcome of ["enabled", "disabled", "failed"] as const) {
    test(`a staging account check with outcome ${outcome} cannot publish after ${invalidate}`, async (t) => {
      const profile = deferred<unknown>();
      const screen = setup(true, [], { getAccountProfile: () => profile.promise });
      if (invalidate !== "unmount") t.after(() => screen.h.unmount());
      const scan = screen.camera()!.props.onBarcodeScanned({ data: "https://staging.nyxid.ai/login/device?user_code=ABCD-EFGH" });
      if (invalidate === "session") screen.changeSession();
      if (invalidate === "background") screen.appState("background");
      if (invalidate === "unmount") screen.h.unmount();
      if (outcome === "failed") profile.reject(new Error("offline"));
      else profile.resolve({ capabilities: { enabled_features: outcome === "enabled" ? ["auth:staging-qr-login"] : [] } });
      await scan;
      if (invalidate !== "unmount") await screen.h.settle();
      assert.deepEqual(screen.routed, []);
      assert.equal(screen.error(), undefined);
    });
  }
}
