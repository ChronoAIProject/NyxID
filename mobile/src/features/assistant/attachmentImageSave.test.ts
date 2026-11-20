import assert from "node:assert/strict";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { deferred, hookHarness } from "./assistantHookTestHarness";
import { attachmentSchema } from "./assistantTypes";
import { ApiError } from "../../lib/api/ApiError";
import type * as Save from "./saveAttachmentImage";
import type * as Cache from "./attachmentCache";

type Element = { type: unknown; props: Record<string, unknown> };
const nodes = (root: Element): Element[] => [
  root,
  ...[root.props.children]
    .flat(Infinity)
    .flatMap((child) =>
      child && typeof child === "object" && "props" in child
        ? nodes(child as Element)
        : [],
    ),
];
const text = (root: Element): string =>
  [root.props.children]
    .flat(Infinity)
    .map((child) =>
      typeof child === "string"
        ? child
        : child && typeof child === "object" && "props" in child
          ? text(child as Element)
          : "",
    )
    .join(" ");
function setup(platform = "ios", expired = false, image = true, processCache?: typeof Cache) {
  const calls: { kind: string; args: unknown[] }[] = [];
  let dimensions = { width: 390, height: 844, fontScale: 1 };
  let state = "active";
  let epoch = 1;
  let focused = true;
  let authenticated = true;
  let app: (state: string) => void = () => {};
  let share: () => Promise<{
    action: string;
    activityType?: string;
  }> = async () => ({
    action: "sharedAction",
    activityType: "com.apple.UIKit.activity.SaveToCameraRoll",
  });
  let picker: () => Promise<Dir> = async () =>
    new Dir("content://folder/Pictures");
  let writeFailure = false;
  let sweeps = 0;
  let download: () => Promise<Uint8Array> = async () =>
    new Uint8Array([1, 2, 3]);
  class Dir {
    exists = false;
    uri: string;
    name: string | undefined = "Pictures";
    constructor(...args: unknown[]) {
      this.uri = args.join("/");
    }
    create() {
      this.exists = true;
      calls.push({ kind: "cache", args: [this.uri] });
    }
    list() {
      sweeps++;
      return [];
    }
    delete() {
      this.exists = false;
      calls.push({ kind: "delete", args: [this.uri] });
    }
    static pickDirectoryAsync() {
      calls.push({ kind: "pick", args: [] });
      return picker();
    }
    createFile(name: string, type: string) {
      calls.push({ kind: "destination", args: [name, type] });
      return new File(this, name);
    }
  }
  class File {
    uri: string;
    constructor(directory: Dir, name: string) {
      this.uri = `${directory.uri}/${name}`;
    }
    create() {}
    write(bytes: Uint8Array) {
      if (this.uri.startsWith("content:") && writeFailure)
        throw new Error("private path");
      calls.push({ kind: "write", args: [this.uri, Array.from(bytes)] });
    }
    delete() {
      calls.push({ kind: "partial-delete", args: [this.uri] });
    }
  }
  const harness = hookHarness<Element>({
    "react/jsx-runtime": {
      jsx: (type: unknown, props: Element["props"]) => ({ type, props }),
      jsxs: (type: unknown, props: Element["props"]) => ({ type, props }),
    },
    "react-native": {
      ...Object.fromEntries(
        ["View", "Pressable", "Image", "ActivityIndicator"].map((n) => [n, n]),
      ),
      useWindowDimensions: () => dimensions,
      findNodeHandle: () => 42,
      Platform: { OS: platform },
      StyleSheet: { create: (s: unknown) => s },
      AppState: {
        get currentState() {
          return state;
        },
        addEventListener: (_event: string, listener: typeof app) => {
          app = listener;
          return { remove() {} };
        },
      },
      Share: {
        dismissedAction: "dismissedAction",
        sharedAction: "sharedAction",
        share: async (...args: unknown[]) => {
          calls.push({ kind: "share", args });
          return share();
        },
      },
    },
    "expo-file-system": {
      Directory: Dir,
      File,
      Paths: { cache: "file:///private/cache" },
    },
    "expo-sharing": {
      isAvailableAsync: async () => true,
      shareAsync: async (...args: unknown[]) =>
        calls.push({ kind: "document", args }),
    },
    "@react-navigation/native": { useIsFocused: () => focused },
    "lucide-react-native": {
      Download: "Download",
      ExternalLink: "ExternalLink",
      RotateCcw: "RotateCcw",
      FileText: "FileText",
      ImageIcon: "ImageIcon",
    },
    "../../components/workspace/WorkspaceUI": { Label: "Label" },
    "../../lib/api/http": {
      requestAuthenticatedBytes: async (...args: unknown[]) => {
        calls.push({ kind: "download", args });
        return download();
      },
    },
    "../../lib/auth/authEpoch": { getAuthEpoch: () => epoch },
    "../../theme/ThemeContext": { useTheme: () => ({ colors: {} }) },
    "../auth/AuthSessionContext": {
      useAuthSession: () => ({ isAuthenticated: authenticated }),
    },
    "./AssistantImagePreview": { AssistantImagePreview: "Preview" },
  });
  const save = harness.load(
    fileURLToPath(new URL("./saveAttachmentImage.ts", import.meta.url)),
  ) as typeof Save;
  harness.mock("./saveAttachmentImage", save);
  const cache = processCache ?? harness.load(
    fileURLToPath(new URL("./attachmentCache.ts", import.meta.url)),
  ) as typeof Cache;
  harness.mock("./attachmentCache", cache);
  const { AssistantAttachmentView } = harness.load(
    fileURLToPath(new URL("./AssistantAttachmentView.tsx", import.meta.url)),
  ) as { AssistantAttachmentView: (props: unknown) => Element };
  harness.mount(() =>
    AssistantAttachmentView({
      attachment: attachmentSchema.parse({
        id: "image",
        label: "Photo",
        content_type: image ? "image/png" : "text/plain",
        size: 3,
        origin: "tool",
        expired,
      }),
      conversationId: "chat",
    }),
  );
  return {
    harness,
    resize: (fontScale: number) => { dimensions = { ...dimensions, fontScale }; harness.render(); },
    cache,
    calls,
    save,
    sweeps: () => sweeps,
    text: () => text(harness.value()),
    nodes: () => nodes(harness.value()),
    async press(label: string) {
      const node = nodes(harness.value()).find(
        (n) => n.props.accessibilityLabel === label,
      );
      assert.ok(node, label);
      (node.props.onPress as () => void)();
      await harness.settle();
    },
    share(fn: typeof share) {
      share = fn;
    },
    picker(fn: typeof picker) {
      picker = fn;
    },
    folderName(name: string | undefined) {
      picker = async () => {
        const folder = new Dir("content://folder/selected");
        folder.name = name;
        return folder;
      };
    },
    failWrite() {
      writeFailure = true;
    },
    download(fn: typeof download) {
      download = fn;
    },
    async refocus() {
      focused = true;
      harness.render();
      await harness.settle();
    },
    async app(next: string) {
      state = next;
      app(state);
      harness.render();
      await harness.settle();
    },
    async cancel(kind: string) {
      if (kind === "background") {
        state = "background";
        app(state);
      }
      if (kind === "auth") authenticated = false;
      if (kind === "epoch") epoch++;
      if (kind === "blur") focused = false;
      harness.render();
      await harness.settle();
    },
  };
}

test("image thumbnail alone previews; separate 44pt Save uses private authenticated bytes and an anchored iOS sheet", async () => {
  const env = setup();
  await env.harness.settle();
  assert.equal(
    env
      .nodes()
      .filter(
        (n) =>
          n.type === "Pressable" &&
          String(n.props.accessibilityLabel).startsWith("Image from"),
      ).length,
    1,
  );
  await env.press("Image from Photo");
  assert.ok(env.nodes().find((n) => n.type === "Preview")?.props.uri);
  (
    env.nodes().find((n) => n.type === "Preview")!.props.onClose as () => void
  )();
  env.harness.flush();
  const button = env
    .nodes()
    .find((n) => n.props.accessibilityLabel === "Save image")!;
  const style = (
    button.props.style as (s: {
      pressed: boolean;
    }) => Array<{ minHeight?: number }>
  )({ pressed: false });
  assert.ok(style[0]!.minHeight! >= 44);
  await env.press("Save image");
  const share = env.calls.find((c) => c.kind === "share")!;
  assert.match(
    (share.args[0] as { url: string }).url,
    /^file:\/\/\/private\/cache\//,
  );
  assert.equal((share.args[1] as { anchor: number }).anchor, 42);
  assert.equal(env.calls.filter((c) => c.kind === "download").length, 2);
  assert.match(env.text(), /Saved to Photos/);
  assert.equal(
    env.nodes().find((n) => n.type === "Preview")?.props.uri,
    undefined,
  );
  assert.equal(env.calls.filter((c) => c.kind === "delete").length, 1);
  env.harness.unmount();
  assert.equal(env.calls.filter((c) => c.kind === "delete").length, 2);
});

test("iOS save reports Photos, Files, other apps, dismissal and sanitized failures with cleanup", async () => {
  for (const [activity, expected] of [
    ["com.apple.UIKit.activity.SaveToCameraRoll", "Saved to Photos"],
    ["com.apple.DocumentManagerUICore.SaveToFiles", "Saved to Files"],
    ["com.apple.UIKit.activity.Mail", "Shared."],
    ["com.apple.UIKit.activity.CopyToPasteboard", "Image copied."],
    ["some.provider.SaveToDocuments", "Saved to Files."],
    ["some.provider.SaveToFiles", "Saved to Files."],
    [undefined, "Shared."],
    ["dismissed", "Image not saved"],
    ["failure", "Could not save the image"],
  ]) {
    const env = setup();
    await env.harness.settle();
    env.share(async () => {
      if (activity === "failure") throw new Error("/private/secret");
      return {
        action: activity === "dismissed" ? "dismissedAction" : "sharedAction",
        activityType: activity,
      };
    });
    await env.press("Save image");
    assert.ok(env.text().includes(expected!));
    assert.doesNotMatch(env.text(), /secret|com\.apple\.|some\.provider/);
    assert.equal(env.calls.filter((c) => c.kind === "delete").length, 1);
    env.harness.unmount();
  }
});

test("Android saves bytes to the SAF destination, reports cancellation, and removes a failed partial write", async () => {
  for (const outcome of ["success", "cancel", "failure"]) {
    const env = setup("android");
    await env.harness.settle();
    if (outcome === "cancel")
      env.picker(async () => {
        throw { code: "ERR_PICKER_CANCELLED" };
      });
    if (outcome === "failure") env.failWrite();
    await env.press("Save image");
    assert.match(
      env.text(),
      outcome === "success"
        ? /Saved to Pictures/
        : outcome === "cancel"
          ? /Image not saved/
          : /Could not save/,
    );
    assert.equal(env.calls.filter((c) => c.kind === "share").length, 0);
    if (outcome === "success")
      assert.deepEqual(
        env.calls.find(
          (c) => c.kind === "write" && String(c.args[0]).startsWith("content:"),
        )?.args[1],
        [1, 2, 3],
      );
    if (outcome === "failure")
      assert.equal(
        env.calls.filter((c) => c.kind === "partial-delete").length,
        1,
      );
    assert.equal(
      env.calls.filter((c) => c.kind === "delete").length,
      0,
      "SAF Save does not create a plaintext cache copy",
    );
    env.harness.unmount();
  }
});

test("saving is polite and busy; auth, epoch and focus cancellation clean private files and reject late status", async () => {
  for (const kind of ["auth", "epoch", "blur"]) {
    const env = setup();
    await env.harness.settle();
    const sheet = deferred<{ action: string }>();
    env.share(() => sheet.promise);
    await env.press("Save image");
    assert.match(env.text(), /Saving image/);
    assert.equal(
      env.nodes().find((n) => n.props.accessibilityLabel === "Save image")
        ?.props.disabled,
      true,
    );
    assert.ok(
      env.nodes().some((n) => n.props.accessibilityLiveRegion === "polite"),
    );
    await env.cancel(kind);
    assert.equal(env.calls.filter((c) => c.kind === "delete").length, 2);
    sheet.resolve({ action: "sharedAction" });
    await env.harness.settle();
    assert.doesNotMatch(env.text(), /Saved to|Shared\.|Image copied/);
    env.harness.unmount();
  }
});

test("expired images hide Save; failed image loading offers a single Load retry; documents have one Open action", async () => {
  const expired = setup("ios", true);
  await expired.harness.settle();
  assert.equal(
    expired.nodes().some((n) => n.props.accessibilityLabel === "Save image"),
    false,
  );
  assert.equal(expired.calls.length, 0);
  expired.harness.unmount();
  const env = setup();
  await env.harness.settle();
  (env.nodes().find((n) => n.type === "Image")!.props.onError as () => void)();
  env.harness.flush();
  assert.equal(
    env.nodes().filter((n) => n.props.accessibilityLabel === "Load image")
      .length,
    1,
  );
  assert.equal(
    env.nodes().some((n) => n.props.accessibilityLabel === "Save image"),
    false,
  );
  await env.press("Load image");
  assert.ok(env.nodes().find((n) => n.type === "Image"));
  env.harness.unmount();
  const doc = setup("ios", false, false);
  assert.equal(doc.calls.length, 0);
  assert.equal(doc.nodes().filter((n) => n.type === "Pressable").length, 1);
  await doc.press("Open attachment Photo");
  assert.equal(doc.calls.filter((c) => c.kind === "document").length, 1);
  assert.equal(doc.calls.filter((c) => c.kind === "delete").length, 1);
  doc.harness.unmount();
});

test("retention errors during Save hide actions and delete the temporary image", async () => {
  const env = setup();
  await env.harness.settle();
  env.download(async () => {
    throw new ApiError({
      statusCode: 410,
      errorCode: 12101,
      errorKey: "attachment_expired",
      message: "expired",
    });
  });
  await env.press("Save image");
  assert.equal(
    env.nodes().some((n) => n.props.accessibilityLabel === "Save image"),
    false,
  );
  assert.match(env.text(), /Attachment expired/);
  env.harness.unmount();
});

test("background cancels a pending Save download, while a native share target retains its private file until completion", async () => {
  const downloading = setup();
  await downloading.harness.settle();
  const bytes = deferred<Uint8Array>();
  downloading.download(() => bytes.promise);
  await downloading.press("Save image");
  await downloading.cancel("background");
  assert.equal(
    downloading.calls.filter((call) => call.kind === "delete").length,
    1,
  );
  assert.match(downloading.text(), /Image not saved/);
  bytes.resolve(new Uint8Array([1, 2, 3]));
  await downloading.harness.settle();
  assert.equal(
    downloading.calls.filter((call) => call.kind === "share").length,
    0,
  );
  downloading.harness.unmount();
  const sharing = setup();
  await sharing.harness.settle();
  const sheet = deferred<{ action: string; activityType: string }>();
  sharing.share(() => sheet.promise);
  await sharing.press("Save image");
  await sharing.cancel("background");
  assert.equal(
    sharing.calls.filter((call) => call.kind === "delete").length,
    0,
  );
  sheet.resolve({
    action: "sharedAction",
    activityType: "com.apple.UIKit.activity.SaveToCameraRoll",
  });
  await sharing.harness.settle();
  assert.equal(
    sharing.calls.filter((call) => call.kind === "delete").length,
    1,
  );
  assert.match(sharing.text(), /Saved to Photos/);
  sharing.harness.unmount();
});

test("native image saving rejects remote URLs and missing iPad anchors before opening a share sheet", async () => {
  const env = setup();
  await env.harness.settle();
  const input = {
    file: { uri: "https://authenticated.example.test/image" },
    bytes: new Uint8Array([1, 2, 3]),
    name: "image.png",
    contentType: "image/png",
    anchor: 42,
    alive: () => true,
  };
  await assert.rejects(
    env.save.saveAttachmentImage(input),
    /Could not save the image/,
  );
  await assert.rejects(
    env.save.saveAttachmentImage({
      ...input,
      file: { uri: "file:///private/cache/image.png" },
      anchor: null,
    }),
    /Could not save the image/,
  );
  assert.equal(env.calls.filter((call) => call.kind === "share").length, 0);
  env.harness.unmount();
});

test("attachment actions use Download only for Save, RotateCcw for retry and ExternalLink for document Open", async () => {
  const env = setup();
  await env.harness.settle();
  const assertAction = (
    source: ReturnType<typeof setup>,
    label: string,
    icon: string,
  ) => {
    const action = source
      .nodes()
      .find((node) => node.props.accessibilityLabel === label)!;
    const glyphs = nodes(action).filter((node) =>
      ["Download", "RotateCcw", "ExternalLink"].includes(String(node.type)),
    );
    assert.equal(glyphs.length, 1);
    assert.equal(glyphs[0]!.type, icon);
    assert.equal(glyphs[0]!.props.accessible, false);
    assert.equal(glyphs[0]!.props.size, 16);
    const style = (
      typeof action.props.style === "function"
        ? (
            action.props.style as (state: {
              pressed: boolean;
            }) => Array<{ minHeight?: number; minWidth?: number }>
          )({ pressed: false })
        : action.props.style
    ) as Array<{ minHeight?: number; minWidth?: number }>;
    assert.ok(style[0]!.minHeight! >= 44);
    assert.ok(style[0]!.minWidth! >= 44);
  };
  assertAction(env, "Save image", "Download");
  (
    env.nodes().find((node) => node.type === "Image")!.props
      .onError as () => void
  )();
  env.harness.flush();
  assertAction(env, "Load image", "RotateCcw");
  await env.press("Load image");
  assertAction(env, "Save image", "Download");
  env.harness.unmount();
  const document = setup("ios", false, false);
  assertAction(document, "Open attachment Photo", "ExternalLink");
  assert.equal(
    document.nodes().filter((node) => node.type === "Pressable").length,
    1,
  );
  document.harness.unmount();
});

test("Android Save names an unnamed SAF destination as the selected folder", async () => {
  for (const name of ["", undefined, "   "]) {
    const env = setup("android");
    await env.harness.settle();
    env.folderName(name);
    await env.press("Save image");
    assert.match(env.text(), /Saved to the selected folder\./);
    assert.doesNotMatch(env.text(), /Saved to \.|undefined/);
    assert.equal(
      env.calls.filter((call) => call.kind === "destination").length,
      1,
    );
    assert.equal(env.calls.filter((call) => call.kind === "delete").length, 0);
    env.harness.unmount();
  }
});

for (const boundary of ["download", "share sheet"] as const) {
  test(`inactive during image Save ${boundary} always settles the current operation's status`, async () => {
    for (const outcome of ["success", "cancel", "failure"]) {
      const env = setup();
      await env.harness.settle();
      const bytes = deferred<Uint8Array>();
      const sheet = deferred<{ action: string; activityType?: string }>();
      if (boundary === "download") env.download(() => bytes.promise);
      else env.share(() => sheet.promise);
      if (boundary === "download")
        env.share(async () => {
          if (outcome === "failure") throw new Error("native failure");
          return {
            action: outcome === "cancel" ? "dismissedAction" : "sharedAction",
            activityType: "com.apple.UIKit.activity.SaveToCameraRoll",
          };
        });
      await env.press("Save image");
      assert.match(env.text(), /Saving image/);
      await env.app("inactive");
      if (boundary === "download") bytes.resolve(new Uint8Array([1, 2, 3]));
      else if (outcome === "failure") sheet.reject(new Error("native failure"));
      else
        sheet.resolve({
          action: outcome === "cancel" ? "dismissedAction" : "sharedAction",
          activityType: "com.apple.UIKit.activity.SaveToCameraRoll",
        });
      await env.harness.settle();
      assert.doesNotMatch(env.text(), /Saving image/);
      assert.match(
        env.text(),
        outcome === "success"
          ? /Saved to Photos/
          : outcome === "cancel"
            ? /Image not saved/
            : /Could not save the image/,
      );
      assert.equal(
        env
          .nodes()
          .find((node) => node.props.accessibilityLabel === "Save image")?.props
          .disabled,
        false,
      );
      assert.equal(
        env.calls.filter((call) => call.kind === "delete").length,
        1,
      );
      env.harness.unmount();
    }
  });
}

test("Android Save downloads authenticated bytes directly into SAF without an extra cache directory or write", async () => {
  const env = setup("android");
  await env.harness.settle();
  const start = env.calls.length;
  await env.press("Save image");
  const calls = env.calls.slice(start);
  assert.equal(calls.filter((call) => call.kind === "cache").length, 0);
  assert.deepEqual(
    calls
      .filter((call) => call.kind === "write")
      .map((call) => String(call.args[0]).startsWith("content://")),
    [true],
  );
  assert.equal(calls.filter((call) => call.kind === "download").length, 1);
  assert.match(env.text(), /Saved to Pictures/);
  env.harness.unmount();
});

test("a non-image attachment's entire row is the sole Open target and contains its ExternalLink affordance", async () => {
  const env = setup("ios", false, false);
  const row = env.nodes().find((node) => node.type === "Pressable")!;
  assert.equal(row.props.accessibilityLabel, "Open attachment Photo");
  assert.equal(row.props.accessibilityRole, "button");
  assert.ok(nodes(row).some((node) => node.type === "FileText"));
  assert.ok(nodes(row).some((node) => node.type === "ExternalLink"));
  assert.equal(
    nodes(row).filter((node) => node.type === "Pressable").length,
    1,
  );
  await env.press("Open attachment Photo");
  assert.equal(env.calls.filter((call) => call.kind === "document").length, 1);
  env.harness.unmount();
});

test("attachment mount runs orphan-cache recovery before creating a live preview directory", async () => {
  const env = setup();
  assert.equal(env.sweeps(), 1);
  await env.harness.settle();
  assert.equal(env.calls.filter((call) => call.kind === "cache").length, 1);
  env.harness.unmount();
});

test("Android Save settles while inactive after authenticated download without caching plaintext", async () => {
  const env = setup("android");
  await env.harness.settle();
  const download = deferred<Uint8Array>();
  env.download(() => download.promise);
  const start = env.calls.length;
  await env.press("Save image");
  await env.app("inactive");
  download.resolve(new Uint8Array([1, 2, 3]));
  await env.harness.settle();
  assert.match(env.text(), /Saved to Pictures/);
  assert.doesNotMatch(env.text(), /Saving image/);
  assert.equal(
    env.calls.slice(start).filter((call) => call.kind === "cache").length,
    0,
  );
  env.harness.unmount();
});

test("two attachment views mounting and refocusing list the native cache only once per process", async () => {
  const first = setup();
  await first.harness.settle();
  const second = setup("ios", false, true, first.cache);
  await second.harness.settle();
  await first.cancel("blur");
  await second.cancel("blur");
  await first.refocus();
  await second.refocus();
  assert.equal(first.sweeps() + second.sweeps(), 1);
  assert.ok(first.nodes().some((node) => node.type === "Image"));
  assert.ok(second.nodes().some((node) => node.type === "Image"));
  first.harness.unmount();
  second.harness.unmount();
});

test("document-only attachment row keeps the filename column at intrinsic width", () => {
  const env = setup("ios", false, false);
  const row = env.nodes().find((node) => node.type === "Pressable")!;
  assert.ok(!env.nodes().some((node) => node.type === "Image"));
  const name = nodes(row).find(
    (node) => node.type === "Label" && text(node).trim() === "Photo",
  );
  const size = nodes(row).find(
    (node) => node.type === "Label" && text(node).trim() === "3 B",
  );
  assert.ok(name);
  assert.ok(size);
  const metadata = nodes(row).find(
    (node) => node.type === "View" &&
      [node.props.children].flat(Infinity).includes(name),
  );
  assert.ok(metadata);
  assert.ok([metadata.props.children].flat(Infinity).includes(size));
  const style = flatStyle(metadata.props.style) as {
    flex?: number;
    flexGrow?: number;
    flexShrink?: number;
    flexBasis?: number | string;
    minWidth?: number;
  };
  assert.equal(style.flex, undefined);
  assert.equal(style.flexGrow, 1);
  assert.equal(style.flexShrink, 1);
  assert.equal(style.flexBasis ?? "auto", "auto");
  assert.equal(style.minWidth, 0);
  // Label forwards these Text props to the native filename and size nodes.
  assert.equal(name.props.numberOfLines, 1);
  assert.equal(size.props.numberOfLines, 1);
  env.harness.unmount();
});

test("non-image full-row Open feedback is 0.8 when pressed and 0.5 when unavailable or disabled", async () => {
  const env = setup("ios", false, false);
  const row = () => env.nodes().find((node) => node.type === "Pressable")!;
  const opacity = (pressed: boolean) =>
    flatStyle((
      row().props.style as (state: {
        pressed: boolean;
      }) => { opacity?: number }[]
    )({ pressed })).opacity;
  assert.equal(opacity(false), 1);
  assert.equal(opacity(true), 0.8);
  const loading = deferred<Uint8Array>();
  env.download(() => loading.promise);
  await env.press("Open attachment Photo");
  assert.equal(row().props.disabled, true);
  assert.equal(opacity(false), 0.5);
  assert.equal(opacity(true), 0.5);
  loading.resolve(new Uint8Array([1, 2, 3]));
  await env.harness.settle();
  assert.equal(opacity(false), 1);
  env.harness.unmount();
  const unavailable = setup("ios", true, false);
  const target = unavailable.nodes().find((node) => node.type === "Pressable")!;
  assert.equal(target.props.disabled, true);
  const style = target.props.style as (state: {
    pressed: boolean;
  }) => { opacity?: number }[];
  assert.equal(flatStyle(style({ pressed: true })).opacity, 0.5);
  unavailable.harness.unmount();
});

function flatStyle(style: any): Record<string, any> { return Object.assign({}, ...[style].flat(Infinity).filter(Boolean)); }
test("large text moves sent attachment actions below metadata without squeezing the filename", async () => {
  const x = setup("android");
  await x.harness.settle();
  x.resize(2);
  const filename = x.nodes().find(n => n.type === "Label" && n.props.children === "Photo")!;
  assert.equal(filename.props.numberOfLines, undefined);
  const metadata = x.nodes().find(n => n.type === "View" && Array.isArray(n.props.style) && n.props.style[0]?.flexGrow === 1)!;
  assert.equal(flatStyle(metadata.props.style).flexGrow, 0);
  const action = x.nodes().find(n => n.props.accessibilityLabel === "Save image")!;
  const style = flatStyle((action.props.style as Function)({ pressed: false }));
  assert.equal(style.alignSelf, "flex-end");
  assert.equal(style.flexShrink, 0);
  x.harness.unmount();
});
