import { assistantDraftStore } from "./assistantDraftStore";
import { useEffect, useRef, useState } from "react";
import {
  ActionSheetIOS,
  Alert,
  AppState,
  Linking,
  Platform,
} from "react-native";
import { useIsFocused } from "@react-navigation/native";
import * as DocumentPicker from "expo-document-picker";
import { requireOptionalNativeModule } from "expo";
import type { ImagePickerAsset } from "expo-image-picker";
import { File, Paths } from "expo-file-system";
import { getAuthEpoch } from "../../lib/auth/authEpoch";
import { assistantUploadsApi } from "../../lib/api/assistantUploadsApi";
import {
  AssistantUploadSession,
  type PickedAssistantFile,
} from "./AssistantUploadSession";
import {
  attachmentFileError,
  MAX_ASSISTANT_ATTACHMENT_BYTES,
  MAX_ASSISTANT_ATTACHMENTS,
  isSupportedAttachment,
} from "./assistantUploads";

const cachePrefix = () => `${Paths.cache.uri.replace(/\/+$/, "")}/`;

function disposeFile(file: PickedAssistantFile) {
  // DocumentPicker copies into cache; never remove a user's original document.
  if (!file.uri.startsWith(cachePrefix())) return;
  try {
    const cached = new File(file.uri);
    if (cached.exists) cached.delete();
  } catch {
    /* cache eviction */
  }
}
function nativeFile(file: PickedAssistantFile) {
  if (!file.uri.startsWith("file://"))
    throw new Error("The file could not be copied. Choose it again.");
  const source = new File(file.uri);
  if (!source.exists)
    throw new Error("This file is no longer available. Choose it again.");
  return source;
}
function cancelled() {
  return Object.assign(new Error("Upload cancelled."), { name: "AbortError" });
}
export function useAssistantUploads({
  conversationId,
  groupId,
  agentId,
  generation,
  enabled,
  online,
  reservedMentionHandles,
  retainKey,
}: {
  retainKey?: string;
  conversationId?: string;
  groupId?: string;
  agentId?: string;
  generation: number;
  enabled: boolean;
  online: boolean;
  /** Exact live group member handles; not part of upload ownership. */
  reservedMentionHandles?: readonly string[];
}) {
  const focused = useIsFocused();
  const [foreground, setForeground] = useState(
    AppState.currentState === "active",
  );
  const [, setVersion] = useState(0);
  const [picking, setPicking] = useState(false);
  const [error, setError] = useState<string>();
  const live = useRef({
    enabled,
    focused,
    foreground,
    online,
    reservedMentionHandles,
  });
  live.current = {
    enabled,
    focused,
    foreground,
    online,
    reservedMentionHandles,
  };
  const current = useRef<AssistantUploadSession | undefined>(undefined);
  const epoch = getAuthEpoch();
  const key = JSON.stringify([
    epoch,
    generation,
    agentId ?? null,
    conversationId ?? null,
    groupId ?? null,
    retainKey ?? null,
  ]);
  const context = useRef<
    | { key: string; retainKey?: string; session: AssistantUploadSession }
    | undefined
  >(undefined);
  function createSession() {
    let ctx: AssistantUploadSession;
    const dependencies = {
      current: () => current.current === ctx && epoch === getAuthEpoch(),
      reservedMentionHandles: () => live.current.reservedMentionHandles ?? [],
      available: () =>
        live.current.enabled &&
        live.current.focused &&
        live.current.foreground &&
        AppState.currentState === "active" &&
        live.current.online,
      changed: () => setVersion((value) => value + 1),
      disposeFile,
      fileSize: (file) => nativeFile(file).size,
      readBytes: async (file, signal) => {
        if (signal.aborted) throw cancelled();
        const source = nativeFile(file);
        const size = source.size;
        const sizeError = attachmentFileError(file, size);
        if (sizeError) throw new Error(sizeError);
        const handle = source.open();
        try {
          // Bound the read even if the native provider's size is stale. One
          // extra byte detects a changed copy instead of sending a prefix.
          const bytes = handle.readBytes(
            Math.min(size, MAX_ASSISTANT_ATTACHMENT_BYTES) + 1,
          );
          if (signal.aborted) throw cancelled();
          if (bytes.byteLength !== size)
            throw new Error("The file changed. Choose it again.");
          return bytes;
        } finally {
          handle.close();
        }
      },
      createDraft: (signal) => {
        if (groupId) throw new Error("Open a saved crew to attach files.");
        return assistantUploadsApi.createDraft(agentId, signal);
      },
      upload: (id, file, bytes, signal, onProgress) =>
        assistantUploadsApi.upload(
          { kind: groupId ? "groups" : "conversations", id },
          file,
          bytes,
          signal,
          onProgress,
        ),
      remove: (id, attachmentId) =>
        epoch === getAuthEpoch()
          ? assistantUploadsApi.remove(
              { kind: groupId ? "groups" : "conversations", id },
              attachmentId,
            )
          : Promise.resolve(),
    } satisfies ConstructorParameters<typeof AssistantUploadSession>[0];
    const retained =
      retainKey &&
      context.current?.key !== key &&
      context.current?.retainKey === retainKey
        ? undefined
        : retainKey
          ? assistantDraftStore.get<AssistantUploadSession>(
              `uploads:${retainKey}`,
            )
          : undefined;
    ctx =
      retained && !retained.disposed
        ? retained
        : new AssistantUploadSession(dependencies, groupId ?? conversationId);
    ctx.rebind(dependencies);
    if (retainKey)
      assistantDraftStore.set(`uploads:${retainKey}`, ctx, () => ctx.dispose());
    return ctx;
  }
  if (!context.current || context.current.key !== key)
    context.current = { key, retainKey, session: createSession() };
  const session = context.current.session;
  current.current = session;
  const pendingPicker = useRef<
    | {
        session: AssistantUploadSession;
        epoch: number;
        files: PickedAssistantFile[];
      }
    | undefined
  >(undefined);
  const pickerLock = useRef(false);
  const lifecycle = useRef(0);
  useEffect(() => {
    const operation = ++lifecycle.current;
    current.current = session;
    return () => {
      if (current.current === session) current.current = undefined;
      // Development effect replay keeps the same live session. Invalidate
      // ownership immediately, and dispose after the replay microtask.
      queueMicrotask(() => {
        if (lifecycle.current === operation || current.current !== session) {
          const retain =
            retainKey &&
            epoch === getAuthEpoch() &&
            session.entries.size > 0 &&
            !session.sending &&
            assistantDraftStore.get<AssistantUploadSession>(
              `uploads:${retainKey}`,
            ) === session;
          if (retain) session.interrupt();
          else {
            session.dispose();
            if (
              retainKey &&
              assistantDraftStore.get<AssistantUploadSession>(
                `uploads:${retainKey}`,
              ) === session
            )
              assistantDraftStore.delete(`uploads:${retainKey}`);
          }
          if (pendingPicker.current?.session === session) {
            pendingPicker.current.files.forEach(disposeFile);
            pendingPicker.current = undefined;
          }
        }
      });
    };
  }, [session]);
  useEffect(() => {
    const subscription = AppState.addEventListener("change", (state) => {
      live.current.foreground = state === "active";
      if (state !== "active") session.interrupt();
      setForeground(state === "active");
    });
    return () => subscription.remove();
  }, [session]);
  useEffect(() => {
    setError(undefined);
    if (!enabled || !focused || !foreground || !online) session.interrupt();
  }, [session, enabled, focused, foreground, online]);
  useEffect(() => {
    const pending = pendingPicker.current;
    if (!pending) return;
    if (
      pending.session !== session ||
      current.current !== session ||
      pending.epoch !== getAuthEpoch() ||
      !enabled ||
      !focused ||
      !online
    ) {
      pendingPicker.current = undefined;
      pending.files.forEach(disposeFile);
    } else if (foreground && AppState.currentState === "active") {
      // Android can deliver the picker result before its resume callback.
      // Wait for the active render/effects before starting any upload, so an
      // earlier background effect cannot interrupt the newly accepted files.
      pendingPicker.current = undefined;
      session.add(pending.files);
    }
  });
  async function pick(source: "files" | "photos" | "camera" = "files") {
    if (
      current.current !== session ||
      epoch !== getAuthEpoch() ||
      pickerLock.current ||
      pendingPicker.current ||
      !live.current.enabled ||
      !live.current.focused ||
      !live.current.foreground ||
      AppState.currentState !== "active" ||
      !live.current.online ||
      session.sending ||
      session.entries.size >= MAX_ASSISTANT_ATTACHMENTS
    )
      return;
    pickerLock.current = true;
    setPicking(true);
    setError(undefined);
    let pickedCopies: PickedAssistantFile[] = [];
    let transferred = false;
    try {
      let files: PickedAssistantFile[];
      if (source === "photos" || source === "camera") {
        if (!requireOptionalNativeModule("ExponentImagePicker"))
          throw new Error(
            `${source === "camera" ? "Camera" : "Photos"} needs the updated NyxID app. You can still attach images using Files.`,
          );
        const ImagePicker = await import("expo-image-picker");
        if (
          current.current !== session ||
          epoch !== getAuthEpoch() ||
          !live.current.enabled ||
          !live.current.focused ||
          !live.current.online ||
          AppState.currentState !== "active"
        )
          return;
        const photoOptions = {
          mediaTypes: ["images"],
          allowsMultipleSelection: true,
          selectionLimit: Math.max(
            1,
            MAX_ASSISTANT_ATTACHMENTS - session.entries.size,
          ),
          preferredAssetRepresentationMode:
            ImagePicker.UIImagePickerPreferredAssetRepresentationMode
              .Compatible,
          quality: 1,
        } satisfies Parameters<typeof ImagePicker.launchImageLibraryAsync>[0];
        if (source === "camera") {
          const permission = await ImagePicker.requestCameraPermissionsAsync();
          if (
            current.current !== session ||
            epoch !== getAuthEpoch() ||
            !live.current.enabled ||
            !live.current.focused ||
            !live.current.online
          )
            return;
          if (!permission.granted) {
            setError(
              "Camera access is off. Allow NyxID to use the camera in Settings, or choose Photos or Files.",
            );
            Alert.alert(
              "Allow camera access",
              "NyxID needs camera access to take photos to attach in chats. You can allow access in Settings.",
              [
                { text: "Not now", style: "cancel" },
                {
                  text: "Open Settings",
                  onPress: () => {
                    if (current.current !== session || epoch !== getAuthEpoch())
                      return;
                    void Linking.openSettings().catch(() => {
                      if (
                        current.current === session &&
                        epoch === getAuthEpoch()
                      )
                        setError(
                          "Couldn't open Settings. Open your device's Settings and allow camera access for NyxID.",
                        );
                    });
                  },
                },
              ],
            );
            return;
          }
        }
        // The photo library picker needs access only to the chosen images.
        // Captures enter the same conversion, validation and upload session.
        const result =
          source === "camera"
            ? await ImagePicker.launchCameraAsync({
                mediaTypes: ["images"],
                quality: 1,
              })
            : await ImagePicker.launchImageLibraryAsync(photoOptions);
        if (result.canceled) return;
        files = result.assets.map((asset, index) => ({
          uri: asset.uri,
          // iOS can convert HEIC to JPEG; the original asset's name may still
          // end in .HEIC. Use the cached copy's extension after conversion.
          name: photoFileName(asset, index),
          mimeType: asset.mimeType,
          size: asset.fileSize,
        }));
        pickedCopies = files;
        if (
          current.current !== session ||
          epoch !== getAuthEpoch() ||
          !live.current.enabled ||
          !live.current.focused ||
          !live.current.online
        )
          return;
        for (let index = 0; index < files.length; index++) {
          const file = files[index]!;
          if (isSupportedAttachment(file)) continue;
          // Photos can return HEIC/AVIF even in compatible mode. Convert the
          // selected cache copy; never change the person's library asset.
          if (
            !/\.(heic|heif|avif|tiff?|bmp)$/i.test(file.name) &&
            !/^image\/(heic|heif|avif|tiff|bmp)$/i.test(file.mimeType ?? "")
          )
            continue;
          if (!requireOptionalNativeModule("ExpoImageManipulator"))
            throw new Error(
              "This photo format needs the updated NyxID app. Choose a JPEG or PNG using Files.",
            );
          const { ImageManipulator, SaveFormat } =
            await import("expo-image-manipulator");
          const context = ImageManipulator.manipulate(file.uri);
          try {
            const image = await context.renderAsync();
            try {
              const converted = await image.saveAsync({
                format: SaveFormat.JPEG,
                compress: 0.95,
              });
              files[index] = {
                uri: converted.uri,
                name: file.name.replace(/\.[^.]+$/, "") + ".jpg",
                mimeType: "image/jpeg",
              };
              disposeFile(file);
            } finally {
              image.release();
            }
          } finally {
            context.release();
          }
        }
      } else {
        const result = await DocumentPicker.getDocumentAsync({
          type: "*/*",
          multiple: true,
          copyToCacheDirectory: true,
        });
        if (result.canceled) return;
        files = result.assets.map((asset) => ({
          uri: asset.uri,
          name: asset.name,
          mimeType: asset.mimeType,
          size: asset.size,
        }));
        pickedCopies = files;
      }
      if (
        current.current !== session ||
        epoch !== getAuthEpoch() ||
        !live.current.enabled ||
        !live.current.focused ||
        !live.current.online
      )
        files.forEach(disposeFile);
      else {
        pendingPicker.current = { session, epoch, files };
        transferred = true;
        setVersion((value) => value + 1);
      }
    } catch (cause) {
      if (current.current === session && epoch === getAuthEpoch())
        setError(
          source === "camera"
            ? "The camera couldn't open. It may be unavailable on this device. Try again, or choose Photos or Files."
            : cause instanceof Error
              ? cause.message
              : "The file picker could not open.",
        );
    } finally {
      if (!transferred) pickedCopies.forEach(disposeFile);
      pickerLock.current = false;
      if (current.current) setPicking(false);
    }
  }
  function choose(
    additional:
      | readonly []
      | readonly [{ label: string; onPress: () => void }] = [],
  ) {
    if (
      pickerLock.current ||
      current.current !== session ||
      pendingPicker.current ||
      !live.current.enabled ||
      !live.current.focused ||
      !live.current.online ||
      AppState.currentState !== "active" ||
      session.sending
    )
      return;
    const extra = additional.map((action) => ({
      text: action.label,
      onPress: () => {
        if (
          current.current !== session ||
          epoch !== getAuthEpoch() ||
          !live.current.enabled ||
          !live.current.focused ||
          !live.current.online ||
          AppState.currentState !== "active" ||
          session.sending
        )
          return;
        action.onPress();
      },
    }));
    if (Platform.OS === "ios") {
      ActionSheetIOS.showActionSheetWithOptions(
        {
          title: "Add attachments",
          options: [
            "Camera",
            "Photos",
            "Files",
            ...extra.map((action) => action.text),
            "Cancel",
          ],
          cancelButtonIndex: 3 + extra.length,
        },
        (index) => {
          if (index === 0) void pick("camera");
          if (index === 1) void pick("photos");
          if (index === 2) void pick("files");
          if (index >= 3) extra[index - 3]?.onPress();
        },
      );
    } else {
      // Android Alert supports at most three buttons. Back/outside dismisses
      // this first menu; the second menu keeps Files and connection reachable.
      Alert.alert(
        "Add attachments",
        undefined,
        [
          { text: "Camera", onPress: () => void pick("camera") },
          { text: "Photos", onPress: () => void pick("photos") },
          {
            text: "More options…",
            onPress: () =>
              Alert.alert("More attachment options", undefined, [
                { text: "Files", onPress: () => void pick("files") },
                ...extra,
                { text: "Cancel", style: "cancel" },
              ]),
          },
        ],
        { cancelable: true },
      );
    }
  }
  return {
    session,
    files: [...session.entries.values()],
    blocked: session.blocked,
    picking,
    error,
    notice: session.notice,
    pick,
    choose,
    remove: (key: string) => session.remove(key),
  };
}

function photoFileName(asset: ImagePickerAsset, index: number) {
  const extension = asset.uri.split(/[?#]/)[0]?.match(/\.([a-z0-9]+)$/i)?.[1];
  const original = asset.fileName?.replace(/\.[^.]+$/, "");
  return extension
    ? `${original || `photo-${index + 1}`}.${extension}`
    : asset.fileName || `photo-${index + 1}.jpg`;
}
