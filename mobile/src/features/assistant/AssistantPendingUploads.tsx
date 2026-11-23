import { useAppLanguage } from "../../i18n/LanguageProvider";
import { OverflowScrollView } from "../../components/workspace/ScrollEdgeFades";
import { useEffect, useState } from "react";
import {
  Image,
  Pressable,
  StyleSheet,
  View,
  useWindowDimensions,
} from "react-native";
import { FileText, X } from "lucide-react-native";
import { ActionButton, Label } from "../../components/workspace/WorkspaceUI";
import { useTheme } from "../../theme/ThemeContext";
import { radius } from "../../theme/designTokens";
import type { PendingAssistantUpload } from "./AssistantUploadSession";
import { AssistantImagePreview } from "./AssistantImagePreview";
import { assistantUploadStatus } from "./assistantUploadStatus";
import { AssistantUploadProgress } from "./AssistantUploadProgress";

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
  reselect: boolean;
  choose: () => void;
  dismiss: () => void;
  resume: () => void;
  canResume: boolean;
  retryDiscard?: () => void;
}) {
  const { t } = useAppLanguage();
  const { colors } = useTheme();
  const { fontScale, height } = useWindowDimensions();
  const chipHeight = Math.max(
    56,
    12 + (showMentionHandles ? 62 : 48) * fontScale,
  );
  const maxListHeight = Math.max(
    chipHeight + 8,
    Math.min(height * 0.3, chipHeight * 2.5 + 16),
  );
  const [preview, setPreview] = useState<PendingAssistantUpload>();
  useEffect(() => {
    if (preview && !files.some((row) => row === preview)) setPreview(undefined);
  }, [files, preview]);
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
        <OverflowScrollView
          fadeColor={colors.card}
          nestedScrollEnabled
          persistentScrollbar
          style={[styles.list, { maxHeight: maxListHeight }]}
          keyboardShouldPersistTaps="handled"
          contentContainerStyle={[styles.content, styles.grid]}
        >
          {files.map((row) => {
            // Some native document providers omit MIME metadata. Preserve the
            // image preview without changing an already assigned handle.
            const image =
              !!row.file.uri &&
              !row.error &&
              (row.kind === "image" ||
                /\.(png|jpe?g|gif|webp)$/i.test(row.file.name));
            const status = assistantUploadStatus(row);
            return (
              <View
                key={row.key}
                style={[
                  styles.card,
                  {
                    backgroundColor: colors.cardSoft,
                    borderColor: colors.border,
                    height: chipHeight,
                  },
                ]}
              >
                <View style={styles.media}>
                  {image ? (
                    <Pressable
                      accessibilityRole="button"
                      accessibilityLabel={t("Preview {{name}}", {
                        name: row.file.name,
                      })}
                      onPress={() => setPreview(row)}
                    >
                      <Image
                        source={{ uri: row.file.uri }}
                        style={styles.thumbnail}
                      />
                    </Pressable>
                  ) : (
                    <FileText size={23} color={colors.textSecondary} />
                  )}
                </View>
                <View style={styles.fill}>
                  {showMentionHandles && !row.item ? (
                    <Label
                      style={[styles.handle, { color: colors.primaryOnTint }]}
                    >
                      @{row.handle}
                    </Label>
                  ) : null}
                  <Label numberOfLines={1}>
                    {row.item?.label ?? row.file.name}
                  </Label>
                  {status.text ? (
                    <Label
                      muted
                      numberOfLines={2}
                      style={[
                        styles.status,
                        row.error ? { color: colors.danger } : undefined,
                      ]}
                    >
                      {status.text}
                    </Label>
                  ) : null}
                </View>
                <Pressable
                  accessibilityRole="button"
                  accessibilityLabel={t("Remove {{name}}", {
                    name: row.file.name,
                  })}
                  accessibilityState={{ disabled: disabled || !!row.removing, busy: !!row.removing }}
                  disabled={disabled || row.removing}
                  onPress={() => remove(row.key)}
                  style={styles.close}
                >
                  <X size={18} color={colors.textSecondary} />
                </Pressable>
                <AssistantUploadProgress status={status} />
              </View>
            );
          })}
        </OverflowScrollView>
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
const styles = StyleSheet.create({
  fill: { flex: 1, minWidth: 0 },
  list: { flexGrow: 0, alignSelf: "stretch" },
  grid: { flexDirection: "row", flexWrap: "wrap" },
  content: { gap: 8, padding: 4 },
  card: {
    width: 260,
    maxWidth: "100%",
    flexGrow: 1,
    overflow: "hidden",
    flexDirection: "row",
    alignItems: "center",
    gap: 8,
    paddingLeft: 10,
    paddingVertical: 6,
    borderWidth: 1,
    borderRadius: radius.md,
  },
  media: {
    width: 36,
    height: 36,
    alignItems: "center",
    justifyContent: "center",
  },
  thumbnail: { width: 36, height: 36, borderRadius: 6 },
  handle: { fontSize: 11 },
  status: { fontSize: 11 },
  close: {
    minWidth: 44,
    minHeight: 44,
    alignItems: "center",
    justifyContent: "center",
  },
});
