import { useEffect, useRef, useState } from "react";
import { AppState } from "react-native";
import { useIsFocused } from "@react-navigation/native";
import * as DocumentPicker from "expo-document-picker";
import { File, Paths } from "expo-file-system";
import { getAuthEpoch } from "../../lib/auth/authEpoch";
import { assistantUploadsApi } from "../../lib/api/assistantUploadsApi";
import { assistantApi } from "../../lib/api/assistantApi";
import {
  AssistantUploadSession,
  type PickedAssistantFile,
} from "./AssistantUploadSession";
import {
  attachmentFileError,
  MAX_ASSISTANT_ATTACHMENT_BYTES,
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
  model,
  generation,
  enabled,
  online,
}: {
  conversationId?: string;
  groupId?: string;
  agentId?: string;
  model: string;
  generation: number;
  enabled: boolean;
  online: boolean;
}) {
  const focused = useIsFocused();
  const [foreground, setForeground] = useState(
    AppState.currentState === "active",
  );
  const [, setVersion] = useState(0);
  const [picking, setPicking] = useState(false);
  const [error, setError] = useState<string>();
  const live = useRef({ enabled, focused, foreground, online });
  live.current = { enabled, focused, foreground, online };
  const modelRef = useRef(model);
  modelRef.current = model;
  const current = useRef<AssistantUploadSession | undefined>(undefined);
  const epoch = getAuthEpoch();
  // Model selection preserves the same draft and its pending files.
  const key = JSON.stringify([
    epoch,
    generation,
    agentId ?? null,
    conversationId ?? null,
    groupId ?? null,
  ]);
  const context = useRef<
    { key: string; session: AssistantUploadSession } | undefined
  >(undefined);
  function createSession() {
    let confirmedModel: string | undefined;
    const ctx = new AssistantUploadSession(
      {
        current: () => current.current === ctx && epoch === getAuthEpoch(),
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
          if (groupId) throw new Error("Open a saved group to attach files.");
          return assistantUploadsApi.createDraft(agentId, signal);
        },
        prepareDraft: async (id, signal) => {
          // A PATCH failure retains the durable id and already accepted files.
          while (confirmedModel !== modelRef.current) {
            if (
              current.current !== ctx ||
              epoch !== getAuthEpoch() ||
              signal.aborted
            )
              throw cancelled();
            if (
              !live.current.enabled ||
              !live.current.focused ||
              !live.current.foreground ||
              AppState.currentState !== "active" ||
              !live.current.online
            )
              throw new Error("Reconnect and keep this chat open to send.");
            const requested = modelRef.current;
            if (!requested) return;
            await assistantApi.changeModel(id, requested, signal);
            confirmedModel = requested;
          }
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
      },
      groupId ?? conversationId,
    );
    return ctx;
  }
  if (!context.current || context.current.key !== key)
    context.current = { key, session: createSession() };
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
          session.dispose();
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
  async function pick() {
    if (
      pickerLock.current ||
      pendingPicker.current ||
      !enabled ||
      !focused ||
      !foreground ||
      AppState.currentState !== "active" ||
      !online ||
      session.sending
    )
      return;
    pickerLock.current = true;
    setPicking(true);
    setError(undefined);
    try {
      const result = await DocumentPicker.getDocumentAsync({
        type: "*/*",
        multiple: true,
        copyToCacheDirectory: true,
      });
      if (result.canceled) return;
      const files = result.assets.map((asset) => ({
        uri: asset.uri,
        name: asset.name,
        mimeType: asset.mimeType,
        size: asset.size,
      }));
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
        setVersion((value) => value + 1);
      }
    } catch (cause) {
      if (current.current === session && epoch === getAuthEpoch())
        setError(
          cause instanceof Error
            ? cause.message
            : "The file picker could not open.",
        );
    } finally {
      pickerLock.current = false;
      if (current.current) setPicking(false);
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
    remove: (key: string) => session.remove(key),
  };
}
