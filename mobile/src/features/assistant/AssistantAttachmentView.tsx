import { localizedError } from "../../i18n/localizedError";
import { useLocalizedState } from "../../i18n/useLocalizedState";
import { useAppLanguage } from "../../i18n/LanguageProvider";
import { translateApp } from "../../i18n/appTranslation";
import {
  saveAttachmentImage,
  IMAGE_SAVE_FAILED,
  IMAGE_NOT_SAVED,
} from "./saveAttachmentImage";
import { useCallback, useEffect, useRef, useState } from "react";
import {
  ActivityIndicator,
  AppState,
  Image,
  findNodeHandle,
  Pressable,
  Platform,
  StyleSheet,
  View,
  useWindowDimensions,
} from "react-native";
import { useIsFocused } from "@react-navigation/native";
import { Directory, File, Paths } from "expo-file-system";
import * as Sharing from "expo-sharing";
import {
  Download,
  ExternalLink,
  FileText,
  ImageIcon,
  RotateCcw,
} from "lucide-react-native";
import { Label } from "../../components/workspace/WorkspaceUI";
import { requestAuthenticatedBytes } from "../../lib/api/http";
import { getAuthEpoch } from "../../lib/auth/authEpoch";
import { useTheme } from "../../theme/ThemeContext";
import { radius, TOUCH_TARGET } from "../../theme/designTokens";
import { useAuthSession } from "../auth/AuthSessionContext";
import type { AssistantAttachment } from "./assistantTypes";
import { AssistantImagePreview } from "./AssistantImagePreview";
import { ownAttachmentCache, sweepAttachmentCaches } from "./attachmentCache";
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
  release: () => void;
};

function deleteCache(cache: CachedAttachment | undefined) {
  if (!cache) return;
  cache.release();
  if (!cache.created) return;
  try {
    if (cache.directory.exists) cache.directory.delete();
  } catch {
    // The OS may already have evicted a private cache file.
  }
}

/** Owner-only documents load on demand; visible images load inline; no credential-bearing URLs. */
export function AssistantAttachmentView({
  attachment,
  conversationId,
  groupId,
  captionColor,
}: {
  attachment: AssistantAttachment;
  conversationId?: string;
  groupId?: string;
  captionColor?: string;
}) {
  const { t } = useAppLanguage();
  const { colors } = useTheme();
  const { width, fontScale } = useWindowDimensions();
  const compact = width / Math.max(1, fontScale) < 340;
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
  const scope = `${epoch}:${target?.kind}:${target?.id}:${attachment.id}:${attachment.expired}`;
  const current = useRef({ scope, focused, isAuthenticated });
  current.current = { scope, focused, isAuthenticated };
  const mounted = useRef(false);
  const operation = useRef(0);
  const downloading = useRef<AbortController | undefined>(undefined);
  const cached = useRef<CachedAttachment | undefined>(undefined);
  const savingCache = useRef<CachedAttachment | undefined>(undefined);
  const saveButton = useRef<View>(null);
  const sharingFile = useRef<
    | {
        file?: File;
        scope: string;
        epoch: number;
      }
    | undefined
  >(undefined);
  const busyRef = useRef(false);
  const savingRef = useRef(false);
  const [busy, setBusy] = useState(false);
  const [saving, setSaving] = useState(false);
  const [saveStatus, setSaveStatus] = useLocalizedState<string>();
  const [enlarged, setEnlarged] = useState(false);
  const [foreground, setForeground] = useState(
    AppState.currentState === "active",
  );
  const [preview, setPreview] = useState<string>();
  const [error, setError] = useLocalizedState<string>();
  const [expired, setExpired] = useState(false);
  const image = isPreviewableAttachment(attachment);
  const unavailable = attachment.expired || expired;
  const label = attachment.label || translateApp("Attachment");

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
    savingRef.current = false;
    removeCache();
    deleteCache(savingCache.current);
    savingCache.current = undefined;
    sharingFile.current = undefined;
  }, [removeCache]);

  useEffect(() => {
    mounted.current = true;
    sweepAttachmentCaches();
    setPreview(undefined);
    setEnlarged(false);
    setBusy(false);
    setError(undefined);
    setExpired(false);
    setSaving(false);
    setSaveStatus(undefined);
    return () => {
      mounted.current = false;
      cancel();
    };
  }, [scope, focused, isAuthenticated, cancel]);

  useEffect(() => {
    const subscription = AppState.addEventListener("change", (state) => {
      // Native share targets and Android's folder picker can background this
      // app. Their completion owns cleanup while the same visible owner holds it.
      // Scope/account/navigation cleanup still deletes it immediately.
      setForeground(state === "active");
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
      const wasSaving = savingRef.current;
      cancel();
      setPreview(undefined);
      setEnlarged(false);
      setBusy(false);
      setSaving(false);
      if (wasSaving) setSaveStatus(() => (translateApp(IMAGE_NOT_SAVED)));
    });
    return () => subscription.remove();
  }, [cancel, t]);

  function closePreview() {
    setEnlarged(false);
  }

  async function open(save = false) {
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
    const sameOperation = () =>
      mounted.current &&
      operation.current === request &&
      current.current.scope === scope &&
      current.current.focused &&
      current.current.isAuthenticated &&
      epoch === getAuthEpoch() &&
      !controller.signal.aborted;
    const alive = () => sameOperation() && AppState.currentState === "active";
    busyRef.current = true;
    setBusy(true);
    savingRef.current = save;
    setSaving(save);
    if (save) setSaveStatus(() => (translateApp("Saving image…")));
    setError(undefined);
    let openedCache: CachedAttachment | undefined;
    const removeOpenedCache = () => {
      if (cached.current === openedCache) cached.current = undefined;
      if (savingCache.current === openedCache) savingCache.current = undefined;
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
      if (!(save ? sameOperation() : alive())) return;
      assertReadableAttachment(attachment, bytes.byteLength);
      if (save && Platform.OS === "android") {
        const nativeSave = { scope, epoch };
        sharingFile.current = nativeSave;
        try {
          const result = await saveAttachmentImage({
            bytes,
            name: attachmentCacheName(attachment),
            contentType: attachment.content_type,
            anchor: null,
            alive: sameOperation,
          });
          if (sameOperation()) setSaveStatus(() => (translateApp(result.phrase, result.values)));
        } finally {
          if (sharingFile.current === nativeSave)
            sharingFile.current = undefined;
        }
        return;
      }
      if (!save) removeCache();
      const directory = new Directory(
        Paths.cache,
        `nyx-attachment-${Date.now()}-${++cacheSequence}-${Math.random().toString(36).slice(2)}`,
      );
      const cache: CachedAttachment = {
        directory,
        created: false,
        release: ownAttachmentCache(directory),
      };
      openedCache = cache;
      if (save) savingCache.current = cache;
      else cached.current = cache;
      directory.create();
      cache.created = true;
      const file = new File(directory, attachmentCacheName(attachment));
      cache.file = file;
      file.create();
      file.write(bytes);
      if (!(save ? sameOperation() : alive())) {
        removeOpenedCache();
        return;
      }
      if (save) {
        sharingFile.current = { file, scope, epoch };
        try {
          const result = await saveAttachmentImage({
            file,
            bytes,
            name: attachmentCacheName(attachment),
            contentType: attachment.content_type,
            anchor: findNodeHandle(saveButton.current),
            alive: sameOperation,
          });
          if (sameOperation()) setSaveStatus(() => (translateApp(result.phrase, result.values)));
        } finally {
          removeOpenedCache();
        }
      } else if (image) setPreview(file.uri);
      else {
        const available = await Sharing.isAvailableAsync();
        if (!alive()) {
          removeOpenedCache();
          return;
        }
        if (!available) throw localizedError(ATTACHMENT_UNAVAILABLE_MESSAGE);
        sharingFile.current = { file, scope, epoch };
        try {
          // Native cancellation resolves normally and is not shown as an error.
          await Sharing.shareAsync(file.uri, {
            mimeType: attachment.content_type,
            dialogTitle: translateApp("Open attachment"),
          });
        } finally {
          // An older sharing operation cannot clear a newer operation's file.
          removeOpenedCache();
        }
      }
    } catch (cause) {
      removeOpenedCache();
      if (save ? sameOperation() : alive()) {
        const message = attachmentAccessError(cause);
        if (save) setSaveStatus(() => (translateApp(IMAGE_SAVE_FAILED)));
        else setError(() => (message));
        setExpired(message === translateApp(ATTACHMENT_EXPIRED_MESSAGE));
      }
    } finally {
      if (operation.current === request) {
        downloading.current = undefined;
        busyRef.current = false;
        savingRef.current = false;
        if (mounted.current && current.current.scope === scope) {
          setBusy(false);
          setSaving(false);
        }
      }
    }
  }

  useEffect(() => {
    if (
      image &&
      foreground &&
      focused &&
      isAuthenticated &&
      !attachment.expired
    )
      void open();
    // Fetch is tied to the authenticated visible attachment, never a Markdown URL.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [scope, image, foreground, focused, isAuthenticated]);

  const FileRow = image ? View : Pressable;
  const FileAction = image ? Pressable : View;
  const actionDisabled = busy || unavailable || !target;
  return (
    <View style={styles.container}>
      {image && preview && !unavailable ? (
        <Pressable
          accessibilityRole="button"
          accessibilityLabel={translateApp("Image from {{label}}", {
            label: label,
          })}
          onPress={() => setEnlarged(true)}
          style={{ minHeight: 44 }}
        >
          <Image
            accessibilityLabel={translateApp("Image from {{label}}", {
              label: label,
            })}
            source={{ uri: preview }}
            resizeMode="contain"
            style={{ width: "100%", height: 220, borderRadius: radius.md }}
            onError={() => {
              cancel();
              setPreview(undefined);
              setError(() => (translateApp(ATTACHMENT_UNAVAILABLE_MESSAGE)));
            }}
          />
        </Pressable>
      ) : null}
      {image && busy && !saving ? (
        <Label muted accessibilityLiveRegion="polite">
          {translateApp("Loading image from {{label}}", { label })}
        </Label>
      ) : null}
      <FileRow
        accessibilityRole={image ? undefined : "button"}
        accessibilityLabel={
          image ? undefined : t("Open attachment {{name}}", { name: label })
        }
        accessibilityState={
          image ? undefined : { disabled: actionDisabled, busy }
        }
        disabled={image ? undefined : actionDisabled}
        onPress={image ? undefined : () => void open()}
        style={
          image
            ? [
                styles.file,
                compact && styles.compactFile,
                {
                  backgroundColor: colors.card,
                  borderColor: colors.border,
                  opacity: unavailable ? 0.5 : 1,
                },
              ]
            : ({ pressed }) => [
                styles.file,
                compact && styles.compactFile,
                {
                  backgroundColor: colors.card,
                  borderColor: colors.border,
                  opacity: actionDisabled ? 0.5 : pressed ? 0.8 : 1,
                },
              ]
        }
      >
        {image ? (
          <ImageIcon
            size={18}
            color={colors.textSecondary}
            accessible={false}
          />
        ) : (
          <FileText size={18} color={colors.textSecondary} accessible={false} />
        )}
        <View style={[styles.metadata, compact && styles.compactMetadata]}>
          <Label numberOfLines={compact ? undefined : 1}>{label}</Label>
          {!unavailable && (
            <Label muted numberOfLines={1} style={styles.detail}>
              {(attachment.size < 1024
                ? (!image && attachment.pages != null && attachment.pages > 0 ? (attachment.content_type === "application/pdf" ? (attachment.pages === 1 ? translateApp("{{size}} B · {{pages}} page", { size: attachment.size, pages: attachment.pages }) : translateApp("{{size}} B · {{pages}} pages", { size: attachment.size, pages: attachment.pages })) : (attachment.pages === 1 ? translateApp("{{size}} B · {{pages}} section", { size: attachment.size, pages: attachment.pages }) : translateApp("{{size}} B · {{pages}} sections", { size: attachment.size, pages: attachment.pages }))) : translateApp("{{size}} B", { size: attachment.size })) : (attachment.size < 1024 * 1024
                  ? (!image && attachment.pages != null && attachment.pages > 0 ? (attachment.content_type === "application/pdf" ? (attachment.pages === 1 ? translateApp("{{kilobytes}} KB · {{pages}} page", { kilobytes: Math.ceil(attachment.size / 1024), pages: attachment.pages }) : translateApp("{{kilobytes}} KB · {{pages}} pages", { kilobytes: Math.ceil(attachment.size / 1024), pages: attachment.pages })) : (attachment.pages === 1 ? translateApp("{{kilobytes}} KB · {{pages}} section", { kilobytes: Math.ceil(attachment.size / 1024), pages: attachment.pages }) : translateApp("{{kilobytes}} KB · {{pages}} sections", { kilobytes: Math.ceil(attachment.size / 1024), pages: attachment.pages }))) : translateApp("{{kilobytes}} KB", { kilobytes: Math.ceil(attachment.size / 1024) })) : (!image && attachment.pages != null && attachment.pages > 0 ? (attachment.content_type === "application/pdf" ? (attachment.pages === 1 ? translateApp("{{megabytes}} MB · {{pages}} page", { megabytes: (attachment.size / (1024 * 1024)).toFixed(1), pages: attachment.pages }) : translateApp("{{megabytes}} MB · {{pages}} pages", { megabytes: (attachment.size / (1024 * 1024)).toFixed(1), pages: attachment.pages })) : (attachment.pages === 1 ? translateApp("{{megabytes}} MB · {{pages}} section", { megabytes: (attachment.size / (1024 * 1024)).toFixed(1), pages: attachment.pages }) : translateApp("{{megabytes}} MB · {{pages}} sections", { megabytes: (attachment.size / (1024 * 1024)).toFixed(1), pages: attachment.pages }))) : translateApp("{{megabytes}} MB", { megabytes: (attachment.size / (1024 * 1024)).toFixed(1) }))))}
            </Label>
          )}
        </View>
        {!unavailable && (
          <FileAction
            ref={image ? saveButton : undefined}
            accessibilityRole={image ? "button" : undefined}
            accessibilityLabel={
              image ? t(preview ? "Save image" : "Load image") : undefined
            }
            accessibilityState={
              image ? { disabled: actionDisabled, busy } : undefined
            }
            disabled={image ? actionDisabled : undefined}
            onPress={image ? () => void open(!!preview) : undefined}
            style={
              image
                ? ({ pressed }) => [
                    styles.action,
                    compact && styles.compactAction,
                    {
                      opacity: unavailable ? 0.5 : pressed ? 0.8 : 1,
                    },
                  ]
                : [styles.action, compact && styles.compactAction]
            }
          >
            <Label>
              {t(image ? (preview ? "Save image" : "Load image") : "Open")}
            </Label>
            {busy ? (
              <ActivityIndicator size="small" color={colors.primary} />
            ) : image ? (
              preview ? (
                <Download
                  size={16}
                  color={colors.textMuted}
                  accessible={false}
                />
              ) : (
                <RotateCcw
                  size={16}
                  color={colors.textMuted}
                  accessible={false}
                />
              )
            ) : (
              <ExternalLink
                size={16}
                color={colors.textMuted}
                accessible={false}
              />
            )}
          </FileAction>
        )}
      </FileRow>
      {saveStatus && (
        <Label muted accessibilityLiveRegion="polite" style={styles.detail}>
          {saveStatus}
        </Label>
      )}
      {(unavailable || error) && (
        <Label
          muted
          accessibilityLiveRegion="polite"
          style={[
            styles.detail,
            captionColor ? { color: captionColor } : undefined,
          ]}
        >
          {unavailable ? translateApp(ATTACHMENT_EXPIRED_MESSAGE) : error}
        </Label>
      )}
      {!unavailable && attachment.image_input === "unavailable" && (
        <Label
          muted
          style={[
            styles.detail,
            captionColor ? { color: captionColor } : undefined,
          ]}
        >
          {attachment.image_input_reason === "model_text_only" ||
          attachment.image_input_reason === "image_input_unsupported"
            ? translateApp(
                "This model can’t view images. Choose an image-capable model or describe the image in text.",
              )
            : attachment.image_input_reason === "guest_media_unavailable"
              ? translateApp(
                  "This guest image was saved in this chat but was unavailable to the model in that turn.",
                )
              : attachment.origin === "user_upload"
                ? translateApp(IMAGE_INPUT_UNAVAILABLE_MESSAGE)
                : translateApp("The agent couldn't view this image.")}
        </Label>
      )}
      <AssistantImagePreview
        uri={enlarged ? preview : undefined}
        label={label}
        closeLabel={translateApp("Close attachment preview")}
        onClose={closePreview}
        onError={() => {
          if (cached.current?.file?.uri !== preview) return;
          cancel();
          setPreview(undefined);
          closePreview();
          setError(() => (translateApp(ATTACHMENT_UNAVAILABLE_MESSAGE)));
        }}
      />
    </View>
  );
}

const styles = StyleSheet.create({
  container: { gap: 4, width: "100%", maxWidth: "100%", minWidth: 0 },
  file: {
    minWidth: TOUCH_TARGET,
    flexDirection: "row",
    alignItems: "center",
    gap: 10,
    borderWidth: 1,
    borderRadius: radius.md,
    paddingHorizontal: 12,
    paddingVertical: 8,
    minHeight: TOUCH_TARGET,
  },
  compactFile: { flexDirection: "column", alignItems: "stretch" },
  compactMetadata: { flexGrow: 0 },
  compactAction: { alignSelf: "flex-end", flexShrink: 0, maxWidth: "100%" },
  metadata: { flexGrow: 1, flexShrink: 1, minWidth: 0 },
  action: {
    minHeight: 44,
    minWidth: 44,
    padding: 8,
    gap: 8,
    flexDirection: "row",
    alignItems: "center",
    flexShrink: 1,
  },
  detail: { fontSize: 11, lineHeight: 16 },
});
