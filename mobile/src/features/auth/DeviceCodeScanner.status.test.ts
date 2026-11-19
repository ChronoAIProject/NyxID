import assert from "node:assert/strict";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { hookHarness } from "../assistant/assistantHookTestHarness";
import * as activity from "../../lib/systemStatusActivity";

type Element = { type: unknown; props: Record<string, any> };
function elements(node: Element): Element[] {
  return [node, ...[node.props.children].flat(Infinity).flatMap((child) =>
    child && typeof child === "object" && "props" in child ? elements(child) : [],
  )];
}

function setup(granted: boolean) {
  let permission = { granted, canAskAgain: true };
  const jsx = (type: unknown, props: Element["props"]) => ({ type, props });
  const h = hookHarness<Element>({
    "react/jsx-runtime": { jsx, jsxs: jsx },
    "react-native": {
      View: "View", Text: "Text", Pressable: "Pressable", ActivityIndicator: "ActivityIndicator",
      StyleSheet: { create: (styles: unknown) => styles, absoluteFill: {} },
      useWindowDimensions: () => ({ width: 466, height: 678 }),
      AppState: { addEventListener: () => ({ remove() {} }) },
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
  h.mount(() => module.DeviceCodeScanner({ onCancel() {}, onCode() {}, onManualEntry() {} }));
  return {
    h,
    camera: () => elements(h.value()).find((node) => node.type === "CameraView"),
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
