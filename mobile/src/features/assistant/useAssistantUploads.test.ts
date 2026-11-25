import { buildApiError } from "../../lib/api/responseError";
import { createUploadDraftRecovery } from "./uploadDraftRecovery";
import { createAssistantDraftStore } from "./assistantDraftStore";
import assert from "node:assert/strict";
import test from "node:test";
import { URL, fileURLToPath } from "node:url";
import { deferred, hookHarness } from "./assistantHookTestHarness";
import type { useAssistantUploads } from "./useAssistantUploads";

function memoryDisk() {
  const records = new Map<string, string>();
  return {
    getItem: async (key: string) => records.get(key) ?? null,
    setItem: async (key: string, value: string) => {
      records.set(key, value);
    },
    removeItem: async (key: string) => {
      records.delete(key);
    },
    getAllKeys: async () => [...records.keys()],
    multiRemove: async (keys: string[]) => {
      for (const key of keys) records.delete(key);
    },
  };
}
function harness(
  platform = "android",
  recovery?: {
    disk: ReturnType<typeof memoryDisk>;
    owner: string;
    key: string;
    missing?: boolean;
    upload?: (id: string, name: string) => Promise<{ id: string }>;

  },
) {
  let state = "active";
  let account = recovery?.owner ?? null;
  const durable = createUploadDraftRecovery(recovery?.disk ?? memoryDisk());
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
  const destinations: string[] = [];
  const deletions: string[] = [];
  let uploadCount = 0;
  const props: {
    generation: number;
    enabled: boolean;
    online: boolean;
    retainKey?: string;
    groupId?: string;
    conversationId?: string;
    agentId?: string;
  } = {
    groupId: recovery?.key.startsWith("crew:")
      ? recovery.key.slice(5)
      : undefined,
    conversationId: recovery?.key === privateKey ? "original-chat" : undefined,
    generation: 0,
    enabled: true,
    online: true,
    retainKey: recovery?.key,
  };
  class File {
    constructor(readonly uri: string) {}
    get exists() {
      return !recovery?.missing && !disposed.includes(this.uri);
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
    "./uploadDraftStorage": { uploadDraftStorage: durable },
    "../../lib/auth/authAccount": { getAuthAccountId: () => account },
    "./assistantDraftStore": {
      discardUploadDraft: (owner: string, key: string) =>
        durable.save(owner, key, { rows: [], reselect: false }),
      pendingUploadDiscard: () => undefined,
      assistantDraftStore: createAssistantDraftStore(() => epoch),
    },
    expo: { requireOptionalNativeModule: () => (installed ? {} : null) },
    "react-native": {
      Platform: { OS: platform },
      Keyboard: { dismiss() {} },
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
        upload: async (scope: { id: string }, file: { name: string }) => {
          requests.push("upload");
          destinations.push(scope.id);
          const receipt = await recovery?.upload?.(scope.id, file.name);
          return {
            id: receipt?.id ?? `file-${++uploadCount}` ,
            expired: false,
            label: "note.txt",
            size: 3,
            content_type: "text/plain",
            origin: "user_upload",
          };
        },
        remove: async (_scope: unknown, id: string) => { deletions.push(id); },
      },
    },
  });
  const hook = h.load(
    fileURLToPath(new URL("./useAssistantUploads.ts", import.meta.url)),
  ).useAssistantUploads as typeof useAssistantUploads;
  h.mount(() => hook(props));
  return {
    h,
    durable,
    props,
    requests,
    destinations,
    deletions,
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
        h.flush();
        h.value().sourcePicker?.choices.find(b => b.label === label)?.onPress();
      }
    },
    cancelChooser: () => {
      if (platform === "ios") iosChoice?.(3);
      else { h.flush(); h.value().sourcePicker?.onClose(); }
    },
    choose: (source: "files" | "photos" | "camera") => {
      if (platform === "ios")
        iosChoice?.(source === "camera" ? 0 : source === "photos" ? 1 : 2);
      else {
        h.flush();
        h.value().sourcePicker?.choices.find(b => b.label ===
          (source === "camera" ? "Camera" : source === "photos" ? "Photos" : "Files"))?.onPress();
      }
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
      account = account ? "other-account" : null;
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
      x.h.flush();
      assert.deepEqual(
        Array.from(
          platform === "ios"
            ? x.iosOptions().options
            : x.h.value().sourcePicker?.choices.map((b) => b.label),
        ),
        platform === "ios"
          ? ["Camera", "Photos", "Files", "Cancel"]
          : ["Camera", "Photos", "Files"],
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

test("Android source rows reject captured stale authority and close after context changes", async () => {
  for (const boundary of ["account", "chat", "permission", "offline", "blur"]) {
    const x = harness();
    x.h.value().choose();
    x.h.flush();
    const camera = x.h.value().sourcePicker!.choices[0]!;
    if (boundary === "account") x.changeOwner();
    if (boundary === "chat") x.props.generation++;
    if (boundary === "permission") x.props.enabled = false;
    if (boundary === "offline") x.props.online = false;
    if (boundary === "blur") x.blur();
    x.h.render();
    assert.equal(x.h.value().sourcePicker, undefined);
    camera.onPress();
    x.cameraPermission.resolve({ granted: true });
    await x.h.settle();
    assert.equal(x.calls.length, 0);
    assert.equal(x.requests.length, 0);
    x.h.unmount();
  }
});

test("Android attachment choices use a vertical list and retain Files without nested native dialog buttons", () => {
  const x = harness("android");
  x.h.value().choose();
  x.h.flush();
  assert.deepEqual(Array.from(x.h.value().sourcePicker?.choices.map((choice: { label: string }) => choice.label) ?? []), ["Camera", "Photos", "Files"]);
  assert.equal(x.choices().length, 0, "large labels never enter Android Alert's button row");
  x.h.value().sourcePicker!.onClose();
  x.h.flush();
  assert.equal(x.h.value().sourcePicker, undefined);
  x.h.unmount();
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
    } else { x.h.flush(); assert.equal(x.h.value().sourcePicker?.choices.length, 4); }
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

const privateKey = JSON.stringify(["private", "original-chat"]);
for (const platform of ["android", "ios"]) {
  test(`${platform} reopens an interrupted Files selection in its original chat and clears on dismissal`, async () => {
    const recovery = { disk: memoryDisk(), owner: "owner-A", key: privateKey };
    const old = harness(platform, recovery);
    await old.h.settle();
    void old.h.value().pick("files");
    await old.h.settle();
    assert.equal(old.calls[0]?.source, "files");
    const home = harness(platform, { ...recovery, key: "nyxbot-home" });
    await home.h.settle();
    assert.equal(home.h.value().otherDraft?.name, "Assistant");
    assert.equal(
      (home.h.value().otherDraft?.params as { conversationId: string })
        .conversationId,
      "original-chat",
    );
    assert.equal(home.h.value().session.reselect, false);
    const otherAccount = harness(platform, { ...recovery, owner: "owner-B" });
    await otherAccount.h.settle();
    assert.equal(otherAccount.h.value().session.reselect, false);
    const reopened = harness(platform, recovery);
    await reopened.h.settle();
    assert.equal(reopened.h.value().session.reselect, true);
    assert.equal(reopened.requests.length, 0);
    reopened.h.value().session.dismissReselection();
    await reopened.h.settle();
    assert.equal(
      await reopened.durable.load(recovery.owner, recovery.key),
      undefined,
    );
  });
  test(`${platform} ready upload survives process death and cache eviction with its original receipt`, async () => {
    const recovery = {
      disk: memoryDisk(),
      owner: "owner-A",
      key: "crew:crew-1",
    };
    const old = harness(platform, recovery);
    await old.h.settle();
    const pick = old.h.value().pick();
    await old.h.settle();
    old.finish();
    await pick;
    await old.h.settle();
    assert.equal(old.h.value().files[0]?.item?.id, "file-1");
    const cold = harness(platform, { ...recovery, missing: true });
    await cold.h.settle();
    assert.equal(cold.h.value().files[0]?.file.uri, "");
    assert.equal(cold.h.value().files[0]?.item?.id, "file-1");
    assert.equal(cold.requests.length, 0);
    cold.props.agentId = "resolved-agent";
    cold.h.render();
    await cold.h.settle();
    assert.equal(cold.h.value().files[0]?.item?.id, "file-1");
    const refused = await cold.h.value().session.submit(async () => false);
    assert.equal(refused, false);
    assert.equal(
      (await cold.durable.load(recovery.owner, recovery.key))?.rows.length,
      1,
    );
    await cold.h.value().session.submit(async (payload) => {
      assert.equal(
        await cold.durable.load(recovery.owner, recovery.key),
        undefined,
      );
      assert.equal(payload?.conversationId, "crew-1");
      assert.deepEqual(payload?.attachmentIds, ["file-1"]);
      return true;
    });
    assert.equal(
      await cold.durable.load(recovery.owner, recovery.key),
      undefined,
    );
  });
}
test("cold local copies resume only on request, and an evicted unuploaded file requests reselection", async () => {
  const r = { disk: memoryDisk(), owner: "owner-A", key: privateKey };
  await createUploadDraftRecovery(r.disk).save(r.owner, r.key, {
    rows: [
      {
        handle: "file-1",
        kind: "file",
        file: { path: "DocumentPicker/abc.txt", name: "abc.txt" },
      },
    ],
    reselect: false,
  });
  const missing = harness("ios", { ...r, missing: true });
  await missing.h.settle();
  assert.equal(missing.h.value().files.length, 0);
  assert.equal(missing.h.value().session.reselect, true);
  const cold = harness("android", r);
  await cold.h.settle();
  assert.equal(cold.h.value().files[0]?.resumable, true);
  assert.equal(cold.requests.length, 0);
  cold.h.value().session.resume();
  await cold.h.settle();
  assert.equal(cold.h.value().files[0]?.item?.id, "file-1");
  cold.h.value().remove(cold.h.value().files[0]!.key);
  await cold.h.settle();
  assert.equal(await cold.durable.load(r.owner, r.key), undefined);
});

test("discarding an inaccessible draft at Home reveals the next draft and preserves other accounts", async () => {
  const disk = memoryDisk();
  const durable = createUploadDraftRecovery(disk);
  const marker = { rows: [], reselect: true };
  await durable.save("A", privateKey, marker);
  await durable.save("A", "crew:remaining", marker);
  await durable.save("B", privateKey, marker);
  const home = harness("ios", { disk, owner: "A", key: "nyxbot-home" });
  await home.h.settle();
  assert.equal(home.h.value().otherDraft?.key, privateKey);
  await home.h.value().discardOtherDraft();
  await home.h.settle();
  assert.equal(home.h.value().otherDraft?.key, "crew:remaining");
  assert.equal((await durable.load("B", privateKey))?.reselect, true);
  await durable.clear("A");
  assert.equal(await durable.other("A"), undefined);
  assert.ok(await durable.other("B"));
});

test("an expired upload receipt requires a cached copy or reselection", async () => {
  const disk = memoryDisk();
  const marker = {
    rows: [
      {
        handle: "file-1",
        kind: "file" as const,
        file: { name: "note.txt", path: "DocumentPicker/note.txt" },
        item: {
          id: "expired-id",
          size: 3,
          label: "note.txt",
          content_type: "text/plain",
          expired: true,
        },
      },
    ],
    reselect: false,
  };
  await createUploadDraftRecovery(disk).save("A", privateKey, marker);
  const r = { disk, owner: "A", key: privateKey };
  const local = harness("android", r);
  await local.h.settle();
  assert.equal(local.h.value().files[0]?.resumable, true);
  assert.equal(local.h.value().files[0]?.needsReupload, true);
  const evicted = harness("ios", { ...r, missing: true });
  await evicted.h.settle();
  assert.equal(evicted.h.value().files[0]?.needsReupload, true);
  assert.match(evicted.h.value().files[0]?.error ?? "", /Choose it again/);
  assert.equal(evicted.h.value().session.blocked, true);
});

test("text-only send clears an interrupted picker marker and a refused send restores it", async () => {
  const r = { disk: memoryDisk(), owner: "A", key: privateKey };
  const durable = createUploadDraftRecovery(r.disk);
  await durable.save(r.owner, r.key, { rows: [], reselect: true });
  const cold = harness("android", r);
  await cold.h.settle();
  await cold.h.value().session.submit(async () => false);
  assert.equal((await cold.durable.load(r.owner, r.key))?.reselect, true);
  await cold.h.value().session.submit(async () => {
    assert.equal(await cold.durable.load(r.owner, r.key), undefined);
    return true;
  });
  assert.equal(await cold.durable.load(r.owner, r.key), undefined);
});

test("the serialized conversation survives process death during the first of two queued uploads", async () => {
  const r = {
    disk: memoryDisk(),
    owner: "owner-A",
    key: JSON.stringify(["private", "specialist", "fresh-composer"]),
  };
  const held = deferred<{ id: string }>();
  const warm = harness("android", { ...r, upload: () => held.promise });
  await warm.h.settle();
  const name = (n: string) => ({ name: n, uri: `file:///cache/${n}`, size: 3 });
  warm.h.value().session.add([name("one.txt"), name("two.txt")]);
  await warm.h.settle();
  assert.deepEqual(warm.requests, ["draft", "upload"]);

  // A fresh instance sees only serialized data. The first POST is still held.
  const cold = harness("android", r);
  await cold.h.settle();
  assert.ok(cold.h.value().files.every((row) => row.resumable));
  cold.h.value().session.resume();
  await cold.h.settle();
  assert.deepEqual(cold.requests, ["upload", "upload"]);
  assert.deepEqual(cold.destinations, ["draft-1", "draft-1"]);
  held.resolve({ id: "late-old-process-receipt" });
  warm.h.unmount();
  cold.h.unmount();
});

test("a retained crew upload survives optimistic input generations and refusal", async () => {
  const r = { disk: memoryDisk(), owner: "owner-A", key: "crew:crew-id" };
  const warm = harness("android", r);
  await warm.h.settle();
  warm.h.value().session.add([{ uri: "file:///cache/a.txt", name: "a.txt", size: 3 }]);
  await warm.h.settle();
  const session = warm.h.value().session;
  const held = deferred<boolean>();
  const submission = session.submit(() => held.promise);
  warm.props.generation++;
  warm.h.render();
  assert.equal(warm.h.value().session, session);
  assert.equal(session.entries.size, 0);
  held.resolve(false);
  assert.equal(await submission, false);
  warm.props.generation++;
  warm.h.render();
  await warm.h.settle();
  assert.equal(warm.h.value().files[0]?.item?.id, "file-1");
  assert.deepEqual(warm.deletions, []);
  assert.deepEqual(warm.disposed, []);
  warm.h.unmount();
});

test("real receipts survive cold restore and are invalidated only by a definite live bind refusal", async () => {
  for (const [status, body] of [
    [410, { error: "attachment_expired", error_code: 12101, message: "Attachment expired per retention policy" }],
    [404, { error: "not_found", error_code: 1003, message: "Not found: Attachment not found" }],
  ] as const) {
    const r = { disk: memoryDisk(), owner: "owner-A", key: privateKey };
    const warm = harness("ios", r);
    await warm.h.settle();
    warm.h.value().session.add([{ uri: "file:///cache/a.txt", name: "a.txt", size: 3 }]);
    await warm.h.settle();
    assert.equal((await warm.durable.load(r.owner, r.key))?.rows[0]?.item?.expired, false);
    warm.h.unmount();

    // The live policy changed after the receipt was recorded.
    const cold = harness("ios", { ...r, upload: async () => ({ id: "fresh-id" }) });
    await cold.h.settle();
    let sends = 0;
    assert.equal(await cold.h.value().session.submit(async (uploads) => {
      sends++;
      assert.deepEqual(uploads?.attachmentIds, ["file-1"]);
      throw buildApiError(body, status);
    }), false);
    assert.deepEqual(cold.requests, []);
    assert.equal((await cold.durable.load(r.owner, r.key))?.rows[0]?.needsReupload, true);
    await assert.rejects(cold.h.value().session.submit(async () => { sends++; return true; }));
    assert.equal(sends, 1);

    const evicted = harness("android", { ...r, missing: true });
    await evicted.h.settle();
    assert.match(evicted.h.value().files[0]?.error ?? "", /Choose it again/);
    assert.equal(evicted.h.value().files[0]?.resumable, false);
    evicted.h.unmount();

    cold.h.value().session.resume();
    await cold.h.settle();
    assert.deepEqual(cold.deletions, ["file-1"]);
    assert.equal(await cold.h.value().session.submit(async (uploads) => {
      assert.deepEqual(uploads?.attachmentIds, ["fresh-id"]);
      return true;
    }), true);
    cold.h.unmount();
  }
});

test("a model-only draft restores the model before voice or upload creates its conversation", async () => {
  const r = { disk: memoryDisk(), owner: "A", key: JSON.stringify(["private", "specialist", "model-first"]) };
  const warm = harness("ios", r);
  await warm.h.settle();
  warm.blur();
  warm.h.render();
  await warm.h.value().session.selectModel("nyxagent/research");
  warm.h.unmount();
  const cold = harness("android", r);
  await cold.h.settle();
  assert.equal(cold.h.value().session.model, "nyxagent/research");
  const id = await cold.h.value().session.ensureConversation();
  cold.h.value().session.add([{ name: "a.txt", uri: "file:///cache/a.txt", size: 3 }]);
  await cold.h.settle();
  assert.deepEqual(cold.requests, ["draft", "upload"]);
  assert.deepEqual(cold.destinations, [id]);
  assert.equal((await cold.durable.load(r.owner, r.key))?.model, "nyxagent/research");
  assert.equal((await cold.durable.load(r.owner, r.key))?.conversationId, id);
  cold.h.unmount();
});

test("a failed model selection cannot leak through a concurrent serialized attachment checkpoint", async () => {
  const disk = memoryDisk();
  const put = disk.setItem;
  const writingModel = deferred<void>();
  const failWrite = deferred<void>();
  disk.setItem = async (key, value) => {
    if (JSON.parse(value).model) {
      writingModel.resolve();
      await failWrite.promise;
    }
    await put(key, value);
  };
  const held = deferred<{ id: string }>();
  const r = { disk, owner: "A", key: JSON.stringify(["private", "specialist", "race"]), upload: () => held.promise };
  const warm = harness("android", r);
  await warm.h.settle();
  const session = warm.h.value().session;
  session.add([{ name: "a.txt", uri: "file:///cache/a.txt", size: 3 }]);
  await warm.h.settle();
  const choice = session.selectModel("nyxagent/research");
  await writingModel.promise;
  held.resolve({ id: "ready" });
  await warm.h.settle();
  failWrite.reject(new Error("Storage refused the model checkpoint"));
  await assert.rejects(choice);
  await warm.h.settle();
  const persisted = await warm.durable.load(r.owner, r.key);
  assert.equal(session.model, undefined);
  assert.equal(persisted?.model, undefined);
  assert.equal(persisted?.rows[0]?.item?.id, "ready");
  warm.h.unmount();
});
