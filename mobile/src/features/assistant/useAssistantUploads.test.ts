import { createAssistantDraftStore } from "./assistantDraftStore";
import assert from "node:assert/strict";
import test from "node:test";
import { URL, fileURLToPath } from "node:url";
import { deferred, hookHarness } from "./assistantHookTestHarness";
import type { useAssistantUploads } from "./useAssistantUploads";

function harness(platform = "android") {
  let state = "active";
  let epoch = 1;
  let focused = true;
  const listeners = new Set<(state: string) => void>();
  const picker = deferred<unknown>();
  const photoPicker = deferred<unknown>();
  const cameraPicker = deferred<unknown>();
  const cameraPermission = deferred<{ granted: boolean }>();
  let permissionRequests = 0;
  let settingsOpened = 0;
  let iosOptions: any;
  let alertTitle: unknown;
  const calls: { source: string; options: any }[] = [];
  let choices: { text: string; onPress?: () => void }[] = [];
  let iosChoice: ((index: number) => void) | undefined;
  let conversionError = false;
  let installed = true;
  const disposed: string[] = [];
  const requests: string[] = [];
  const props: {
    generation: number;
    enabled: boolean;
    online: boolean;
    retainKey?: string;
  } = { generation: 0, enabled: true, online: true };
  class File {
    constructor(readonly uri: string) {}
    get exists() {
      return !disposed.includes(this.uri);
    }
    size = 3;
    delete() {
      disposed.push(this.uri);
    }
    open() {
      return { readBytes: () => new Uint8Array([1, 2, 3]), close() {} };
    }
  }
  const h = hookHarness<ReturnType<typeof useAssistantUploads>>({
    "./assistantDraftStore": {
      assistantDraftStore: createAssistantDraftStore(() => epoch),
    },
    expo: { requireOptionalNativeModule: () => (installed ? {} : null) },
    "react-native": {
      Platform: { OS: platform },
      Alert: {
        alert: (title: unknown, _: unknown, buttons: typeof choices) => {
          alertTitle = title;
          choices = buttons;
        },
      },
      ActionSheetIOS: {
        showActionSheetWithOptions: (
          options: unknown,
          cb: typeof iosChoice,
        ) => {
          iosOptions = options;
          iosChoice = cb;
        },
      },
      Linking: {
        openSettings: async () => {
          settingsOpened++;
        },
      },
      AppState: {
        get currentState() {
          return state;
        },
        addEventListener: (_: string, cb: (state: string) => void) => {
          listeners.add(cb);
          return { remove: () => listeners.delete(cb) };
        },
      },
    },
    "@react-navigation/native": { useIsFocused: () => focused },
    "expo-document-picker": {
      getDocumentAsync: (options: unknown) => {
        calls.push({ source: "files", options });
        return picker.promise;
      },
    },
    "expo-image-picker": {
      requestCameraPermissionsAsync: () => {
        permissionRequests++;
        return cameraPermission.promise;
      },
      launchCameraAsync: (options: unknown) => {
        calls.push({ source: "camera", options });
        return cameraPicker.promise;
      },
      UIImagePickerPreferredAssetRepresentationMode: {
        Compatible: "compatible",
      },
      launchImageLibraryAsync: (options: unknown) => {
        calls.push({ source: "photos", options });
        return photoPicker.promise;
      },
    },
    "expo-image-manipulator": {
      SaveFormat: { JPEG: "jpeg" },
      ImageManipulator: {
        manipulate: () => ({
          renderAsync: async () => {
            if (conversionError) throw new Error("Could not convert photo.");
            return {
              saveAsync: async () => ({ uri: "file:///cache/converted.jpg" }),
              release() {},
            };
          },
          release() {},
        }),
      },
    },
    "expo-file-system": { File, Paths: { cache: { uri: "file:///cache/" } } },
    "../../lib/auth/authEpoch": { getAuthEpoch: () => epoch },
    "../../lib/api/assistantUploadsApi": {
      assistantUploadsApi: {
        createDraft: async () => {
          requests.push("draft");
          return "draft-1";
        },
        upload: async () => {
          requests.push("upload");
          return {
            id: "file-1",
            label: "note.txt",
            size: 3,
            content_type: "text/plain",
            origin: "user_upload",
          };
        },
        remove: async () => {},
      },
    },
  });
  const hook = h.load(
    fileURLToPath(new URL("./useAssistantUploads.ts", import.meta.url)),
  ).useAssistantUploads as typeof useAssistantUploads;
  h.mount(() => hook(props));
  return {
    h,
    props,
    requests,
    disposed,
    calls,
    picker,
    photoPicker,
    cameraPicker,
    cameraPermission,
    permissionRequests: () => permissionRequests,
    settingsOpened: () => settingsOpened,
    choices: () => choices,
    iosOptions: () => iosOptions,
    alertTitle: () => alertTitle,
    chooseExtra: (label: string) => {
      if (platform === "ios")
        iosChoice?.((iosOptions as any).options.indexOf(label));
      else {
        choices.find((b) => b.text === "More options…")?.onPress?.();
        choices.find((b) => b.text === label)?.onPress?.();
      }
    },
    cancelChooser: () => {
      if (platform === "ios") iosChoice?.(3);
      else choices.find((b) => b.text === "Cancel")?.onPress?.();
    },
    choose: (source: "files" | "photos" | "camera") => {
      if (platform !== "ios" && source === "files")
        choices.find((b) => b.text === "More options…")?.onPress?.();
      if (platform === "ios")
        iosChoice?.(source === "camera" ? 0 : source === "photos" ? 1 : 2);
      else
        choices
          .find(
            (button) =>
              button.text ===
              (source === "camera"
                ? "Camera"
                : source === "photos"
                  ? "Photos"
                  : "Files"),
          )
          ?.onPress?.();
    },
    conversionError: () => {
      conversionError = true;
    },
    removeNativeModules: () => {
      installed = false;
    },
    emit: (next: string) => {
      state = next;
      for (const cb of listeners) cb(next);
    },
    changeOwner: () => {
      epoch++;
    },
    blur: () => {
      focused = false;
    },
    finish: () =>
      picker.resolve({
        canceled: false,
        assets: [
          {
            uri: "file:///cache/DocumentPicker/note.txt",
            name: "note.txt",
            mimeType: "text/plain",
            size: 3,
          },
        ],
      }),
  };
}

test("Android result before resume retains the file but starts no work until active effects commit", async () => {
  const x = harness();
  const picking = x.h.value().pick();
  x.emit("background");
  x.h.flush();
  x.finish();
  await picking;
  await x.h.settle();
  assert.deepEqual(x.requests, []);
  assert.deepEqual(x.disposed, []);
  assert.equal(x.h.value().files.length, 0);
  x.emit("active");
  await x.h.settle();
  assert.deepEqual(x.requests, ["draft", "upload"]);
  assert.equal(x.h.value().files[0]?.progress, 100);
  assert.equal(x.h.value().files[0]?.error, undefined);
  assert.deepEqual(x.disposed, []);
  x.h.unmount();
});

test("plus chooser routes Camera, Photos and Files to their respective native pickers on both platforms", async () => {
  for (const platform of ["ios", "android"]) {
    for (const source of ["camera", "photos", "files"] as const) {
      const x = harness(platform);
      x.h.value().choose();
      assert.equal(x.calls.length, 0);
      assert.deepEqual(
        Array.from(
          platform === "ios"
            ? x.iosOptions().options
            : x.choices().map((b) => b.text),
        ),
        platform === "ios"
          ? ["Camera", "Photos", "Files", "Cancel"]
          : ["Camera", "Photos", "More options…"],
      );
      x.choose(source);
      x.cameraPermission.resolve({ granted: true });
      await x.h.settle();
      assert.equal(x.calls[0]?.source, source);
      assert.equal(x.calls.length, 1);
      if (source === "photos") {
        assert.deepEqual(Array.from(x.calls[0]!.options.mediaTypes), [
          "images",
        ]);
        assert.equal(x.calls[0]!.options.selectionLimit, 10);
        x.photoPicker.resolve({ canceled: true });
      } else if (source === "camera")
        x.cameraPicker.resolve({ canceled: true });
      else x.picker.resolve({ canceled: true });
      await x.h.settle();
      assert.deepEqual(x.requests, []);
      x.h.unmount();
    }
  }
});

test("late chooser actions cannot open a picker for an obsolete account, chat or disabled composer", async () => {
  for (const invalidate of ["account", "chat", "permission", "unmount"]) {
    const x = harness();
    x.h.value().choose();
    if (invalidate === "account") x.changeOwner();
    if (invalidate === "chat") x.props.generation++;
    if (invalidate === "permission") x.props.enabled = false;
    if (invalidate === "unmount") x.h.unmount();
    else x.h.render();
    x.choose("photos");
    assert.equal(x.calls.length, 0, invalidate);
    if (invalidate !== "unmount") x.h.unmount();
  }
});

test("Photos waits for foreground, converts HEIC and reuses the upload session", async () => {
  const x = harness();
  const picked = x.h.value().pick("photos");
  await x.h.settle();
  x.emit("background");
  x.h.flush();
  x.photoPicker.resolve({
    canceled: false,
    assets: [
      {
        uri: "file:///cache/photo.heic",
        fileName: "Camera.HEIC",
        mimeType: "image/heic",
        fileSize: 3,
      },
    ],
  });
  await picked;
  await x.h.settle();
  assert.deepEqual(x.requests, []);
  assert.deepEqual(x.disposed, ["file:///cache/photo.heic"]);
  x.emit("active");
  await x.h.settle();
  assert.deepEqual(x.requests, ["draft", "upload"]);
  assert.equal(x.h.value().files[0]?.file.name, "Camera.jpg");
  assert.equal(x.h.value().files[0]?.file.mimeType, "image/jpeg");
  x.h.unmount();
});

test("photo results and converted cache copies cannot cross account or focus boundaries", async () => {
  for (const change of ["account", "focus"]) {
    const x = harness();
    const picked = x.h.value().pick("photos");
    await x.h.settle();
    if (change === "account") x.changeOwner();
    else x.blur();
    x.h.render();
    x.photoPicker.resolve({
      canceled: false,
      assets: [
        {
          uri: "file:///cache/photo.heic",
          fileName: "photo.heic",
          mimeType: "image/heic",
          fileSize: 3,
        },
      ],
    });
    await picked;
    await x.h.settle();
    assert.deepEqual(x.requests, []);
    assert.deepEqual(x.disposed, ["file:///cache/photo.heic"]);
    x.h.unmount();
  }
});

test("account changes while loading the photo module cannot open its native picker", async () => {
  const x = harness();
  const picking = x.h.value().pick("photos");
  x.changeOwner();
  x.h.render();
  await picking;
  await x.h.settle();
  assert.deepEqual(x.calls, []);
  assert.deepEqual(x.requests, []);
  x.h.unmount();
});

test("failed conversion disposes every selected cache copy and reports a recoverable error", async () => {
  const x = harness();
  x.conversionError();
  const picked = x.h.value().pick("photos");
  x.photoPicker.resolve({
    canceled: false,
    assets: [
      {
        uri: "file:///cache/photo.heic",
        fileName: "photo.heic",
        mimeType: "image/heic",
        fileSize: 3,
      },
    ],
  });
  await picked;
  await x.h.settle();
  assert.deepEqual(x.requests, []);
  assert.deepEqual(x.disposed, ["file:///cache/photo.heic"]);
  assert.match(x.h.value().error!, /picker could not open/);
  assert.equal(x.h.value().picking, false);
  x.h.unmount();
});

test("an older native build reports Photos availability without breaking Files or chat startup", async () => {
  const x = harness();
  x.removeNativeModules();
  await x.h.value().pick("photos");
  await x.h.settle();
  assert.match(x.h.value().error!, /updated NyxID app/);
  assert.equal(x.calls.length, 0);
  assert.equal(x.h.value().picking, false);
  const file = x.h.value().pick("files");
  x.finish();
  await file;
  await x.h.settle();
  assert.deepEqual(x.requests, ["draft", "upload"]);
  x.h.unmount();
});

test("resume before result and a batched background/result/resume each upload exactly once", async () => {
  for (const commitBackground of [true, false]) {
    const x = harness();
    const picking = x.h.value().pick();
    x.emit("background");
    if (commitBackground) x.h.flush();
    x.emit("active");
    x.finish();
    await picking;
    await x.h.settle();
    assert.deepEqual(x.requests, ["draft", "upload"]);
    assert.equal(x.h.value().files[0]?.error, undefined);
    x.h.unmount();
  }
});

test("pending picker copies never cross owner/chat/focus/permission/connectivity or unmount boundaries", async () => {
  for (const invalidation of [
    "owner",
    "generation",
    "focus",
    "enabled",
    "online",
    "unmount",
  ] as const) {
    const x = harness();
    const picking = x.h.value().pick();
    x.emit("background");
    x.h.flush();
    x.finish();
    await picking;
    await x.h.settle();
    if (invalidation === "owner") x.changeOwner();
    else if (invalidation === "generation") x.props.generation++;
    else if (invalidation === "focus") x.blur();
    else if (invalidation === "enabled") x.props.enabled = false;
    else if (invalidation === "online") x.props.online = false;
    if (invalidation === "unmount") {
      x.h.unmount();
      await new Promise<void>((r) => setImmediate(r));
    } else {
      x.h.render();
      x.emit("active");
      await x.h.settle();
    }
    assert.deepEqual(x.requests, [], invalidation);
    assert.deepEqual(
      x.disposed,
      ["file:///cache/DocumentPicker/note.txt"],
      invalidation,
    );
    if (invalidation !== "unmount") x.h.unmount();
  }
});

test("home navigation retains thread attachments and restores their exact upload session", async () => {
  const x = harness();
  x.props.retainKey = "original-thread";
  x.h.render();
  const picking = x.h.value().pick();
  x.finish();
  await picking;
  await x.h.settle();
  const original = x.h.value().session;
  assert.equal(original.entries.size, 1);
  x.props.retainKey = "nyxbot-home";
  x.h.render();
  await x.h.settle();
  assert.equal(x.h.value().files.length, 0);
  assert.equal(original.disposed, false);
  x.props.retainKey = "original-thread";
  x.h.render();
  await x.h.settle();
  assert.equal(x.h.value().session, original);
  assert.equal(x.h.value().files.length, 1);
  assert.equal(x.disposed.length, 0);
  x.changeOwner();
  x.h.render();
  await x.h.settle();
  assert.equal(original.disposed, true);
  assert.equal(x.h.value().files.length, 0);
  x.h.unmount();
});

test("camera permission is requested only on use; denied access links to Settings", async () => {
  for (const platform of ["ios", "android"]) {
    const x = harness(platform);
    x.h.value().choose();
    assert.equal(x.permissionRequests(), 0);
    x.cancelChooser();
    assert.deepEqual(x.calls, []);
    const picked = x.h.value().pick("camera");
    await x.h.settle();
    assert.equal(x.permissionRequests(), 1);
    x.cameraPermission.resolve({ granted: false });
    await picked;
    await x.h.settle();
    assert.equal(x.alertTitle(), "Allow camera access");
    assert.match(x.h.value().error!, /Camera access is off/);
    assert.equal(x.h.value().picking, false);
    x.choices().find((b) => b.text === "Open Settings")!.onPress!();
    await x.h.settle();
    assert.equal(x.settingsOpened(), 1);
    assert.deepEqual(x.calls, []);
    assert.deepEqual(x.requests, []);
    x.h.unmount();
  }
});

test("camera cancel and unavailable hardware leave a responsive composer without uploads", async () => {
  for (const unavailable of [false, true]) {
    const x = harness("ios");
    const picked = x.h.value().pick("camera");
    x.cameraPermission.resolve({ granted: true });
    await x.h.settle();
    if (unavailable)
      x.cameraPicker.reject(new Error("Camera not available on simulator"));
    else x.cameraPicker.resolve({ canceled: true });
    await picked;
    await x.h.settle();
    assert.deepEqual(x.requests, []);
    assert.equal(x.h.value().files.length, 0);
    assert.equal(x.h.value().picking, false);
    if (unavailable)
      assert.match(x.h.value().error!, /camera.*unavailable.*Photos or Files/);
    else assert.equal(x.h.value().error, undefined);
    x.h.unmount();
  }
});

test("camera captures share photo conversion, validation and the single upload session", async () => {
  for (const name of ["capture.heic", "capture.jpg", "capture.exe"]) {
    const x = harness();
    const picked = x.h.value().pick("camera");
    x.cameraPermission.resolve({ granted: true });
    await x.h.settle();
    assert.deepEqual(Array.from(x.calls[0]!.options.mediaTypes), ["images"]);
    x.cameraPicker.resolve({
      canceled: false,
      assets: [
        {
          uri: `file:///cache/${name}`,
          fileName: name,
          mimeType: name.endsWith("heic")
            ? "image/heic"
            : name.endsWith("jpg")
              ? "image/jpeg"
              : "application/octet-stream",
          fileSize: 3,
        },
      ],
    });
    await picked;
    await x.h.settle();
    if (name.endsWith("exe")) {
      assert.deepEqual(x.requests, []);
      assert.ok(x.h.value().files[0]?.error);
    } else {
      assert.deepEqual(x.requests, ["draft", "upload"]);
      assert.equal(x.h.value().files[0]?.file.mimeType, "image/jpeg");
      assert.equal(x.h.value().files[0]?.progress, 100);
    }
    x.h.unmount();
  }
});

test("stale camera permission and capture results cannot cross account or chat boundaries", async () => {
  for (const stage of ["permission", "capture"]) {
    for (const change of ["account", "chat", "focus"]) {
      const x = harness();
      const picked = x.h.value().pick("camera");
      if (stage === "capture") {
        x.cameraPermission.resolve({ granted: true });
        await x.h.settle();
      }
      if (change === "account") x.changeOwner();
      else if (change === "chat") x.props.generation++;
      else x.blur();
      x.h.render();
      x.cameraPermission.resolve({ granted: true });
      x.cameraPicker.resolve({
        canceled: false,
        assets: [
          {
            uri: "file:///cache/capture.jpg",
            mimeType: "image/jpeg",
            fileSize: 3,
          },
        ],
      });
      await picked;
      await x.h.settle();
      assert.deepEqual(x.requests, []);
      assert.equal(x.calls.length, stage === "capture" ? 1 : 0);
      if (stage === "capture")
        assert.deepEqual(x.disposed, ["file:///cache/capture.jpg"]);
      x.h.unmount();
    }
  }
});

test("camera results keep the shared count and size limits", async () => {
  for (const oversized of [false, true]) {
    const x = harness();
    const picked = x.h.value().pick("camera");
    x.cameraPermission.resolve({ granted: true });
    await x.h.settle();
    x.cameraPicker.resolve({
      canceled: false,
      assets: Array.from({ length: oversized ? 1 : 11 }, (_, i) => ({
        uri: `file:///cache/capture-${i}.jpg`,
        mimeType: "image/jpeg",
        fileSize: oversized ? 100 * 1024 * 1024 : 3,
      })),
    });
    await picked;
    await x.h.settle();
    if (oversized) {
      assert.deepEqual(x.requests, []);
      assert.ok(x.h.value().files[0]?.error);
    } else {
      assert.equal(x.h.value().files.length, 10);
      const cameraCalls = x.calls.length;
      await x.h.value().pick("camera");
      assert.equal(
        x.calls.length,
        cameraCalls,
        "full composer cannot open the camera",
      );
    }
    x.h.unmount();
  }
});

test("connection chooser action retains Camera/Photos/Files and rejects stale account or chat callbacks", () => {
  for (const platform of ["ios", "android"]) {
    const x = harness(platform);
    let connections = 0;
    x.h
      .value()
      .choose([{ label: "Use a connection", onPress: () => connections++ }]);
    if (platform === "ios") {
      assert.deepEqual(Array.from(x.iosOptions().options), [
        "Camera",
        "Photos",
        "Files",
        "Use a connection",
        "Cancel",
      ]);
      assert.equal(x.iosOptions().cancelButtonIndex, 4);
    } else assert.equal(x.choices().length, 3);
    x.chooseExtra("Use a connection");
    assert.equal(connections, 1);
    assert.equal(x.calls.length, 0);
    x.h
      .value()
      .choose([{ label: "Use a connection", onPress: () => connections++ }]);
    x.props.enabled = false;
    x.h.render();
    x.chooseExtra("Use a connection");
    assert.equal(connections, 1);
    x.h.unmount();
  }
});
