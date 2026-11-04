import assert from "node:assert/strict";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { hookHarness } from "./assistantHookTestHarness";

type Element = { type: unknown; props: Record<string, any> };
const nodes = (n: any): Element[] =>
  n?.props ? [n, ...[n.props.children].flat(Infinity).flatMap(nodes)] : [];

function setup() {
  const removed: string[] = [];
  const file = {
    key: "draft-image",
    handle: "image-1",
    kind: "image",
    file: { name: "Draft.png", uri: "file:///private/draft.png" },
    progress: 100,
    item: { label: "Draft.png" },
  };
  const props = {
    files: [file],
    disabled: false,
    remove: (key: string) => removed.push(key),
  };
  const h = hookHarness<Element>({
    "react/jsx-runtime": {
      Fragment: "Fragment",
      jsx: (type: unknown, props: any) => ({ type, props }),
      jsxs: (type: unknown, props: any) => ({ type, props }),
    },
    "react-native": {
      View: "View",
      Image: "Image",
      Pressable: "Pressable",
      ActivityIndicator: "Spinner",
      StyleSheet: { create: (s: unknown) => s },
    },
    "lucide-react-native": { Check: "Check", FileText: "FileText", X: "X" },
    "../../components/workspace/ScrollEdgeFades": {
      OverflowScrollView: "ScrollView",
    },
    "../../components/workspace/WorkspaceUI": { Label: "Label" },
    "../../theme/ThemeContext": { useTheme: () => ({ colors: {} }) },
    "./AssistantImagePreview": { AssistantImagePreview: "Preview" },
  });
  const C = h.load(
    fileURLToPath(new URL("./AssistantPendingUploads.tsx", import.meta.url)),
  ).AssistantPendingUploads as (props: any) => Element;
  h.mount(() => C(props));
  const all = () => nodes(h.value());
  const preview = () => all().find((n) => n.type === "Preview")!;
  return { h, props, all, preview, removed, file };
}

test("draft image preview uses the same dismissible viewer and closing preserves the draft upload", () => {
  const x = setup();
  assert.equal(x.preview().props.uri, undefined);
  x.all()
    .find((n) => n.props.accessibilityLabel === "Preview Draft.png")!
    .props.onPress();
  x.h.flush();
  assert.equal(x.preview().props.uri, x.file.file.uri);
  assert.equal(x.preview().props.closeLabel, "Close file preview");
  x.preview().props.onClose();
  x.h.flush();
  assert.equal(x.preview().props.uri, undefined);
  assert.equal(x.props.files[0], x.file);
  assert.deepEqual(x.removed, []);
  assert.ok(
    x.all().some((n) => n.props.accessibilityLabel === "Preview Draft.png"),
  );
  x.h.unmount();
});

test("removing or switching the current draft image closes its preview", () => {
  const x = setup();
  x.all()
    .find((n) => n.props.accessibilityLabel === "Preview Draft.png")!
    .props.onPress();
  x.h.flush();
  assert.equal(x.preview().props.uri, x.file.file.uri);
  x.props.files = [];
  x.h.render();
  assert.equal(x.preview().props.uri, undefined);
  assert.deepEqual(x.removed, []);
  x.h.unmount();
});
