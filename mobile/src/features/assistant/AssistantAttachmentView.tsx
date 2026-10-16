import { useCallback, useEffect, useRef, useState } from "react";
import {
  ActivityIndicator,
  AppState,
  Image,
  Modal,
  Pressable,
  StyleSheet,
  View,
} from "react-native";
import { useIsFocused } from "@react-navigation/native";
import { SafeAreaView } from "react-native-safe-area-context";
import { Directory, File, Paths } from "expo-file-system";
import * as Sharing from "expo-sharing";
import { Download, FileText, ImageIcon, X } from "lucide-react-native";
import { Label } from "../../components/workspace/WorkspaceUI";
import { requestAuthenticatedBytes } from "../../lib/api/http";
import { getAuthEpoch } from "../../lib/auth/authEpoch";
import { useTheme } from "../../theme/ThemeContext";
import { radius, TOUCH_TARGET } from "../../theme/designTokens";
import { useAuthSession } from "../auth/AuthSessionContext";
import type { AssistantAttachment } from "./assistantTypes";
import { assistantAttachmentPath } from "./assistantUploads";
import {
  ATTACHMENT_EXPIRED_MESSAGE,
  ATTACHMENT_UNAVAILABLE_MESSAGE,
  IMAGE_INPUT_UNAVAILABLE_MESSAGE,
  assertReadableAttachment,
  attachmentAccessError,
  attachmentCacheName,
  isPreviewableAttachment,
} from "./assistantAttachmentAccess";

let cacheSequence = 0;

type CachedAttachment = {
  directory: Directory;
  created: boolean;
  file?: File;
};

function deleteCache(cache: CachedAttachment | undefined) {
  if (!cache?.created) return;
  try {
    if (cache.directory.exists) cache.directory.delete();
  } catch {
    // The OS may already have evicted a private cache file.
  }
}

/** Owner-only attachment reads happen on demand; no credential-bearing URLs. */
export function AssistantAttachmentView({
  attachment,
  conversationId,
  groupId,
}: {
  attachment: AssistantAttachment;
  conversationId?: string;
  groupId?: string;
}) {
  const { colors } = useTheme();
  const { isAuthenticated } = useAuthSession();
  const focused = useIsFocused();
  const epoch = getAuthEpoch();
  // Keep groups and private conversations separate even for identical IDs.
  const target =
    groupId && !conversationId
      ? { kind: "groups" as const, id: groupId }
      : conversationId && !groupId
        ? { kind: "conversations" as const, id: conversationId }
        : undefined;
  const scope = `${epoch}:${target?.kind}:${target?.id}:${attachment.id}`;
  const current = useRef({ scope, focused, isAuthenticated });
  current.current = { scope, focused, isAuthenticated };
  const mounted = useRef(false);
  const operation = useRef(0);
  const downloading = useRef<AbortController | undefined>(undefined);
  const cached = useRef<CachedAttachment | undefined>(undefined);
  const sharingFile = useRef<
    | {
        file: File;
        scope: string;
        epoch: number;
      }
    | undefined
  >(undefined);
  const busyRef = useRef(false);
  const [busy, setBusy] = useState(false);
  const [preview, setPreview] = useState<string>();
  const [error, setError] = useState<string>();
  const [expired, setExpired] = useState(false);
  const image = isPreviewableAttachment(attachment);
  const unavailable = attachment.expired || expired;
  const label = attachment.label || "Attachment";

  const removeCache = useCallback(() => {
    const cache = cached.current;
    cached.current = undefined;
    if (sharingFile.current?.file === cache?.file)
      sharingFile.current = undefined;
    deleteCache(cache);
  }, []);

  const cancel = useCallback(() => {
    operation.current += 1;
    downloading.current?.abort();
    downloading.current = undefined;
    busyRef.current = false;
    removeCache();
  }, [removeCache]);

  useEffect(() => {
    mounted.current = true;
    setPreview(undefined);
    setBusy(false);
    setError(undefined);
    setExpired(false);
    return () => {
      mounted.current = false;
      cancel();
    };
  }, [scope, focused, isAuthenticated, cancel]);

  useEffect(() => {
    const subscription = AppState.addEventListener("change", (state) => {
      // An external share target can background this app before it finishes
      // reading the file. Its own shareAsync completion owns that cleanup.
      // Scope/account/navigation cleanup still deletes it immediately.
      if (state !== "background") return;
      const sharing = sharingFile.current;
      if (
        sharing &&
        sharing.scope === current.current.scope &&
        sharing.epoch === getAuthEpoch() &&
        current.current.focused &&
        current.current.isAuthenticated
      )
        return;
      cancel();
      setPreview(undefined);
      setBusy(false);
    });
    return () => subscription.remove();
  }, [cancel]);

  function closePreview() {
    cancel();
    setPreview(undefined);
    setBusy(false);
  }

  async function open() {
    if (
      busyRef.current ||
      !target ||
      unavailable ||
      !focused ||
      !isAuthenticated ||
      AppState.currentState !== "active"
    )
      return;
    const request = ++operation.current;
    const controller = new AbortController();
    downloading.current = controller;
    const alive = () =>
      mounted.current &&
      operation.current === request &&
      current.current.scope === scope &&
      current.current.focused &&
      current.current.isAuthenticated &&
      epoch === getAuthEpoch() &&
      AppState.currentState === "active" &&
      !controller.signal.aborted;
    busyRef.current = true;
    setBusy(true);
    setError(undefined);
    let openedCache: CachedAttachment | undefined;
    const removeOpenedCache = () => {
      if (cached.current === openedCache) cached.current = undefined;
      if (sharingFile.current?.file === openedCache?.file)
        sharingFile.current = undefined;
      deleteCache(openedCache);
    };
    try {
      assertReadableAttachment(attachment);
      const bytes = await requestAuthenticatedBytes(
        assistantAttachmentPath(target, attachment.id),
        controller.signal,
      );
      if (!alive()) return;
      assertReadableAttachment(attachment, bytes.byteLength);
      removeCache();
      const directory = new Directory(
        Paths.cache,
        `nyx-attachment-${Date.now()}-${++cacheSequence}-${Math.random().toString(36).slice(2)}`,
      );
      const cache: CachedAttachment = { directory, created: false };
      openedCache = cache;
      cached.current = cache;
      directory.create();
      cache.created = true;
      const file = new File(directory, attachmentCacheName(attachment));
      cache.file = file;
      file.create();
      file.write(bytes);
      if (!alive()) {
        removeOpenedCache();
        return;
      }
      if (image) setPreview(file.uri);
      else {
        const available = await Sharing.isAvailableAsync();
        if (!alive()) {
          removeOpenedCache();
          return;
        }
        if (!available) throw new Error(ATTACHMENT_UNAVAILABLE_MESSAGE);
        sharingFile.current = { file, scope, epoch };
        try {
          // Native cancellation resolves normally and is not shown as an error.
          await Sharing.shareAsync(file.uri, {
            mimeType: attachment.content_type,
            dialogTitle: "Open attachment",
          });
        } finally {
          // An older sharing operation cannot clear a newer operation's file.
          removeOpenedCache();
        }
      }
    } catch (cause) {
      removeOpenedCache();
      if (alive()) {
        const message = attachmentAccessError(cause);
        setError(message);
        setExpired(message === ATTACHMENT_EXPIRED_MESSAGE);
      }
    } finally {
      if (operation.current === request) {
        downloading.current = undefined;
        busyRef.current = false;
        if (mounted.current && current.current.scope === scope) setBusy(false);
      }
    }
  }

  return (
    <View style={styles.container}>
      <Pressable
        accessibilityRole="button"
        accessibilityLabel={`${image ? "Preview" : "Open"} attachment ${label}`}
        accessibilityState={{
          disabled: busy || unavailable || !target,
          busy,
        }}
        disabled={busy || unavailable || !target}
        onPress={() => void open()}
        style={({ pressed }) => [
          styles.file,
          {
            backgroundColor: colors.card,
            borderColor: colors.border,
            opacity: unavailable ? 0.5 : pressed ? 0.8 : 1,
          },
        ]}
      >
        {image ? (
          <ImageIcon size={18} color={colors.textSecondary} />
        ) : (
          <FileText size={18} color={colors.textSecondary} />
        )}
        <View style={styles.metadata}>
          <Label numberOfLines={1}>{label}</Label>
          {!unavailable && (
            <Label muted style={styles.detail}>
              {attachment.size < 1024
                ? `${attachment.size} B`
                : attachment.size < 1024 * 1024
                  ? `${Math.ceil(attachment.size / 1024)} KB`
                  : `${(attachment.size / (1024 * 1024)).toFixed(1)} MB`}
              {!image && attachment.pages != null && attachment.pages > 0
                ? ` · ${attachment.pages} ${attachment.content_type === "application/pdf" ? (attachment.pages === 1 ? "page" : "pages") : attachment.pages === 1 ? "section" : "sections"}`
                : ""}
            </Label>
          )}
        </View>
        {busy ? (
          <ActivityIndicator size="small" color={colors.primary} />
        ) : !unavailable ? (
          <Download size={16} color={colors.textMuted} />
        ) : null}
      </Pressable>
      {(unavailable || error) && (
        <Label muted accessibilityLiveRegion="polite" style={styles.detail}>
          {unavailable ? ATTACHMENT_EXPIRED_MESSAGE : error}
        </Label>
      )}
      {!unavailable && attachment.image_input === "unavailable" && (
        <Label muted style={styles.detail}>
          {IMAGE_INPUT_UNAVAILABLE_MESSAGE}
        </Label>
      )}
      <Modal
        visible={!!preview}
        animationType="fade"
        onRequestClose={closePreview}
      >
        <SafeAreaView
          style={[styles.previewPage, { backgroundColor: colors.bg }]}
        >
          <View style={[styles.previewHeader, { borderColor: colors.border }]}>
            <Label numberOfLines={1} style={styles.metadata}>
              {label}
            </Label>
            <Pressable
              accessibilityRole="button"
              accessibilityLabel="Close attachment preview"
              onPress={closePreview}
              hitSlop={4}
              style={styles.close}
            >
              <X size={22} color={colors.textPrimary} />
            </Pressable>
          </View>
          {preview && (
            <Image
              accessibilityLabel={label}
              source={{ uri: preview }}
              resizeMode="contain"
              style={styles.previewImage}
              onError={() => {
                if (cached.current?.file?.uri !== preview) return;
                closePreview();
                setError(ATTACHMENT_UNAVAILABLE_MESSAGE);
              }}
            />
          )}
        </SafeAreaView>
      </Modal>
    </View>
  );
}

const styles = StyleSheet.create({
  container: { gap: 4, width: "100%" },
  file: {
    flexDirection: "row",
    alignItems: "center",
    gap: 10,
    borderWidth: 1,
    borderRadius: radius.md,
    paddingHorizontal: 12,
    paddingVertical: 8,
    minHeight: TOUCH_TARGET,
  },
  metadata: { flex: 1, minWidth: 0 },
  detail: { fontSize: 11, lineHeight: 16 },
  previewPage: { flex: 1 },
  previewHeader: {
    flexDirection: "row",
    alignItems: "center",
    gap: 12,
    paddingHorizontal: 16,
    paddingVertical: 8,
    borderBottomWidth: 1,
  },
  close: {
    width: TOUCH_TARGET,
    height: TOUCH_TARGET,
    alignItems: "center",
    justifyContent: "center",
  },
  previewImage: { flex: 1, width: "100%" },
});
