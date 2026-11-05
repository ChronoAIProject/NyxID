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
import { Label } from "../../components/workspace/WorkspaceUI";
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
}: {
  files: PendingAssistantUpload[];
  remove: (key: string) => void;
  disabled: boolean;
  showMentionHandles?: boolean;
}) {
  const { colors } = useTheme();
  const { fontScale } = useWindowDimensions();
  const chipHeight = Math.max(
    56,
    12 + (showMentionHandles ? 62 : 48) * fontScale,
  );
  const [preview, setPreview] = useState<PendingAssistantUpload>();
  useEffect(() => {
    if (preview && !files.some((row) => row === preview)) setPreview(undefined);
  }, [files, preview]);
  return (
    <>
      {files.length ? (
        <OverflowScrollView
          fadeColor={colors.card}
          horizontal
          showsHorizontalScrollIndicator={false}
          style={styles.list}
          keyboardShouldPersistTaps="handled"
          contentContainerStyle={styles.content}
        >
          {files.map((row) => {
            // Some native document providers omit MIME metadata. Preserve the
            // image preview without changing an already assigned handle.
            const image =
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
                {image ? (
                  <Pressable
                    accessibilityRole="button"
                    accessibilityLabel={`Preview ${row.file.name}`}
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
                  accessibilityLabel={`Remove ${row.file.name}`}
                  accessibilityState={{ disabled }}
                  disabled={disabled}
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
        label={preview?.file.name ?? "Image"}
        closeLabel="Close file preview"
        onClose={() => setPreview(undefined)}
      />
    </>
  );
}
const styles = StyleSheet.create({
  fill: { flex: 1, minWidth: 0 },
  list: { flexGrow: 0 },
  content: { gap: 8, padding: 4 },
  card: {
    width: 260,
    overflow: "hidden",
    flexDirection: "row",
    alignItems: "center",
    gap: 8,
    paddingLeft: 10,
    paddingVertical: 6,
    borderWidth: 1,
    borderRadius: radius.md,
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
