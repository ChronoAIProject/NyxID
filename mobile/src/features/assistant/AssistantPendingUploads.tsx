import { useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { Image, Pressable, ScrollView, StyleSheet, View } from "react-native";
import { X } from "lucide-react-native";
import { useAppLanguage } from "../../i18n/LanguageProvider";
import { ActionButton, Label } from "../../components/workspace/WorkspaceUI";
import { useTheme } from "../../theme/ThemeContext";
import { radius } from "../../theme/designTokens";
import { LinearGradient } from "expo-linear-gradient";
import type {
  PendingAssistantUpload,
  PickedAssistantFile,
} from "./AssistantUploadSession";
import { AssistantImagePreview } from "./AssistantImagePreview";
import { AssistantPdfThumbnail } from "./AssistantPdfThumbnail";
import { assistantUploadStatus } from "./assistantUploadStatus";
import { AssistantUploadProgress } from "./AssistantUploadProgress";

import {
  ATTACHMENT_GAP as GAP,
  ATTACHMENT_INSET as INSET,
  attachmentStripGeometry,
} from "./attachmentStripGeometry";
const FILE_TYPES: Record<string, string> = {
  "application/json": "JSON",
  "application/pdf": "PDF",
  "application/vnd.openxmlformats-officedocument.wordprocessingml.document":
    "DOCX",
  "image/gif": "GIF",
  "image/jpeg": "JPG",
  "image/png": "PNG",
  "image/webp": "WEBP",
  "text/csv": "CSV",
  "text/markdown": "MD",
  "text/plain": "TXT",
};

export function AssistantPendingUploads({
  files,
  remove,
  disabled,
  showMentionHandles = true,
  reselect,
  choose,
  dismiss,
  resume,
  canResume,
  retryDiscard,
}: {
  files: PendingAssistantUpload[];
  remove: (key: string) => void;
  disabled: boolean;
  showMentionHandles?: boolean;
  reselect?: boolean;
  choose?: () => void;
  dismiss?: () => void;
  resume?: () => void;
  canResume?: boolean;
  retryDiscard?: () => void;
}) {
  const { t } = useAppLanguage();
  const { colors } = useTheme();
  const scroll = useRef<ScrollView>(null);
  const [width, setWidth] = useState(0);
  const [offset, setOffset] = useState(0);
  const strip = attachmentStripGeometry(width, files.length, offset);
  useEffect(() => {
    if (offset === strip.offset) return;
    setOffset(strip.offset);
    scroll.current?.scrollTo({ x: strip.offset, animated: false });
  }, [offset, strip.offset]);
  const [preview, setPreview] = useState<PendingAssistantUpload>();
  useEffect(() => {
    if (preview && !files.some((row) => row === preview)) setPreview(undefined);
  }, [files, preview]);

  // Scrolling updates range/fades without rebuilding every attachment. Upload
  // notifications supply a new files array, so progress and errors still refresh.
  const tiles = useMemo(
    () =>
      files.map((row) => (
        <PendingUploadCard
          key={row.key}
          row={row}
          width={strip.tile}
          disabled={disabled}
          showMentionHandles={showMentionHandles}
          remove={remove}
          preview={() => setPreview(row)}
        />
      )),
    [files, strip.tile, disabled, showMentionHandles, remove],
  );

  if (retryDiscard)
    return <ActionButton label={t("Retry discard")} onPress={retryDiscard} variant="secondary" />;
  return (
    <>
      {reselect && (
        <View style={styles.content}>
          <Label>
            {t(
              "An attachment selection was interrupted. Choose the file again.",
            )}
          </Label>
          <ActionButton
            label={t("Choose files")}
            onPress={choose}
            disabled={!canResume}
            variant="secondary"
          />
          <ActionButton
            label={t("Dismiss")}
            onPress={dismiss}
            variant="secondary"
          />
        </View>
      )}
      {files.some((row) => row.resumable) && (
        <ActionButton
          label={t("Resume attachment uploads")}
          onPress={resume}
          disabled={!canResume}
          variant="secondary"
        />
      )}
      {files.length ? (
        <View style={styles.listFrame}>
          <View style={styles.viewport}>
            <ScrollView
              ref={scroll}
              testID="composer-attachments"
              horizontal
              showsHorizontalScrollIndicator={false}
              // Explicit width prevents the horizontal content's intrinsic width
              // from expanding the viewport and stranding later remove controls.
              style={[styles.list, { height: strip.tile + 2 * INSET }]}
              onLayout={({ nativeEvent }) => setWidth(nativeEvent.layout.width)}
              onScroll={({ nativeEvent }) =>
                setOffset(
                  attachmentStripGeometry(
                    width,
                    files.length,
                    nativeEvent.contentOffset.x,
                  ).offset,
                )
              }
              scrollEventThrottle={32}
              bounces
              overScrollMode="always"
              nestedScrollEnabled
              keyboardShouldPersistTaps="always"
              contentContainerStyle={styles.content}
            >
              {tiles}
            </ScrollView>
            <View
              pointerEvents="none"
              accessible={false}
              accessibilityElementsHidden
              importantForAccessibility="no-hide-descendants"
              style={StyleSheet.absoluteFill}
            >
              {strip.before ? (
                <LinearGradient
                  testID="attachment-fade-left"
                  colors={[
                    colors.composerBg,
                    `${colors.composerBg}EE`,
                    `${colors.composerBg}00`,
                  ]}
                  locations={[0, 0.3, 1]}
                  start={{ x: 0, y: 0 }}
                  end={{ x: 1, y: 0 }}
                  style={styles.fadeLeft}
                />
              ) : null}
              {strip.after ? (
                <LinearGradient
                  testID="attachment-fade-right"
                  colors={[
                    `${colors.composerBg}00`,
                    `${colors.composerBg}EE`,
                    colors.composerBg,
                  ]}
                  locations={[0, 0.7, 1]}
                  start={{ x: 0, y: 0 }}
                  end={{ x: 1, y: 0 }}
                  style={styles.fadeRight}
                />
              ) : null}
            </View>
          </View>
          {strip.overflow ? (
            <Label muted style={styles.range}>
              {t("Attachments {{start}}–{{end}} of {{count}}", {
                start: strip.start,
                end: strip.end,
                count: files.length,
              })}
            </Label>
          ) : null}
        </View>
      ) : null}
      <AssistantImagePreview
        uri={preview?.file.uri}
        label={preview?.file.name ?? t("Image")}
        closeLabel={t("Close file preview")}
        onClose={() => setPreview(undefined)}
      />
    </>
  );
}

function PendingUploadCard({
  row,
  width,
  disabled,
  showMentionHandles,
  remove,
  preview,
}: {
  row: PendingAssistantUpload;
  width: number;
  disabled: boolean;
  showMentionHandles: boolean;
  remove: (key: string) => void;
  preview: () => void;
}) {
  const { t } = useAppLanguage();
  const { colors } = useTheme();
  const [failedFile, setFailedFile] = useState<PickedAssistantFile>();
  const type =
    (row.item?.content_type ?? row.file.mimeType ?? "")
      .split(";")[0]
      ?.trim()
      .toLowerCase() ?? "";
  const suffix = /\.([^.]+)$/.exec(row.file.name)?.[1]?.toLowerCase();
  // A confirmed server type wins over the picker and filename. Extensions only
  // help providers that omitted MIME metadata or returned a generic type.
  const generic = !type || type === "application/octet-stream";
  const image =
    type.startsWith("image/") ||
    (generic && /^(png|jpe?g|gif|webp)$/.test(suffix ?? ""));
  const pdf = type === "application/pdf" || (generic && suffix === "pdf");
  const safePreview = row.previewReady && failedFile !== row.file;
  const hasThumbnail =
    !!safePreview && (image || (pdf && !!AssistantPdfThumbnail));
  const fileType = FILE_TYPES[type] ?? suffix?.toUpperCase() ?? t("Attachment");
  const name = row.item?.label ?? row.file.name;
  const status = assistantUploadStatus(row);
  const handle = showMentionHandles && !row.item ? `@${row.handle}` : undefined;
  let media: ReactNode;
  if (safePreview && image) {
    media = (
      <Pressable
        accessibilityRole="button"
        accessibilityLabel={t("Preview {{name}}", { name })}
        style={styles.thumbnail}
        onPress={preview}
      >
        <Image
          accessible={false}
          source={{ uri: row.file.uri }}
          resizeMode="cover"
          style={StyleSheet.absoluteFill}
          onError={() => setFailedFile(row.file)}
        />
      </Pressable>
    );
  } else if (safePreview && pdf && AssistantPdfThumbnail) {
    media = (
      <View
        accessible
        accessibilityRole="image"
        accessibilityLabel={t("Preview {{name}}", { name })}
        style={styles.thumbnail}
      >
        <AssistantPdfThumbnail
          uri={row.file.uri}
          style={StyleSheet.absoluteFill}
          onPreviewError={() => setFailedFile(row.file)}
        />
      </View>
    );
  } else {
    media = (
      <View
        accessible
        accessibilityLabel={`${fileType}, ${name}`}
        style={styles.fallback}
      >
        <Label numberOfLines={1} style={styles.fileType}>
          {fileType}
        </Label>
        <Label numberOfLines={2} style={styles.filename}>
          {name}
        </Label>
      </View>
    );
  }
  return (
    <View
      testID={`composer-attachment-${row.key}`}
      style={[
        styles.card,
        {
          width: width || 44,
          backgroundColor: colors.cardSoft,
          borderColor: colors.border,
        },
      ]}
    >
      {hasThumbnail ? media : null}
      <View style={styles.body} pointerEvents={hasThumbnail ? "none" : "auto"}>
        {hasThumbnail ? null : media}
      </View>
      {handle || status.text ? (
        <View style={[styles.caption, { backgroundColor: colors.cardSoft }]}>
          {handle ? (
            <Label
              numberOfLines={1}
              style={[styles.status, { color: colors.primaryOnTint }]}
            >
              {handle}
            </Label>
          ) : null}
          {status.text ? (
            <Label
              muted
              numberOfLines={1}
              accessibilityLabel={status.text}
              accessibilityLiveRegion="polite"
              style={[
                styles.status,
                row.error ? { color: colors.danger } : undefined,
              ]}
            >
              {status.text}
            </Label>
          ) : null}
        </View>
      ) : null}
      <Pressable
        accessibilityRole="button"
        accessibilityLabel={t("Remove {{name}}", { name: row.file.name })}
        accessibilityState={{ disabled: disabled || !!row.removing, busy: !!row.removing }}
        disabled={disabled || row.removing}
        onPress={() => remove(row.key)}
        style={styles.close}
      >
        <View
          style={[
            styles.closeCircle,
            { backgroundColor: colors.card, borderColor: colors.border },
          ]}
        >
          <X size={16} color={colors.textPrimary} />
        </View>
      </Pressable>
      <AssistantUploadProgress status={status} />
    </View>
  );
}

const styles = StyleSheet.create({
  listFrame: { width: "100%", maxWidth: "100%", minWidth: 0, flexShrink: 1 },
  viewport: { width: "100%", maxWidth: "100%", minWidth: 0 },
  list: { width: "100%", maxWidth: "100%", flexGrow: 0, flexShrink: 1 },
  content: {
    flexDirection: "row",
    alignItems: "flex-start",
    gap: GAP,
    padding: INSET,
  },
  card: {
    minWidth: 0,
    aspectRatio: 1,
    overflow: "hidden",
    borderWidth: 1,
    borderRadius: radius.md,
  },
  thumbnail: { ...StyleSheet.absoluteFillObject },
  body: { flex: 1, minHeight: 0 },
  fallback: {
    flex: 1,
    minHeight: 0,
    padding: 8,
    paddingBottom: 4,
    gap: 4,
    overflow: "hidden",
  },
  fileType: { fontSize: 11, paddingRight: 36, minHeight: 24 },
  filename: { flexShrink: 1 },
  caption: { padding: 6, gap: 2 },
  status: { fontSize: 11 },
  close: {
    position: "absolute",
    top: 2,
    right: 2,
    width: 44,
    height: 44,
    alignItems: "center",
    justifyContent: "center",
  },
  closeCircle: {
    width: 28,
    height: 28,
    borderRadius: 14,
    borderWidth: StyleSheet.hairlineWidth,
    alignItems: "center",
    justifyContent: "center",
  },
  fadeLeft: { position: "absolute", top: 0, bottom: 0, left: 0, width: 32 },
  fadeRight: { position: "absolute", top: 0, bottom: 0, right: 0, width: 32 },
  range: { fontSize: 11, textAlign: "center", paddingBottom: 2 },
});
