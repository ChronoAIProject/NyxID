import { useEffect, useState } from "react";
import {
  ActivityIndicator,
  Image,
  Modal,
  Pressable,
  ScrollView,
  StyleSheet,
  View,
} from "react-native";
import { Check, FileText, X } from "lucide-react-native";
import { Label } from "../../components/workspace/WorkspaceUI";
import { useTheme } from "../../theme/ThemeContext";
import { radius } from "../../theme/designTokens";
import type { PendingAssistantUpload } from "./AssistantUploadSession";

export function AssistantPendingUploads({
  files,
  remove,
  disabled,
}: {
  files: PendingAssistantUpload[];
  remove: (key: string) => void;
  disabled: boolean;
}) {
  const { colors } = useTheme();
  const [preview, setPreview] = useState<PendingAssistantUpload>();
  useEffect(() => {
    if (preview && !files.some((row) => row === preview)) setPreview(undefined);
  }, [files, preview]);
  return (
    <>
      {files.length ? (
        <ScrollView
          style={styles.list}
          keyboardShouldPersistTaps="handled"
          contentContainerStyle={styles.content}
        >
          {files.map((row) => {
            const image =
              !row.error && /\.(png|jpe?g|gif|webp)$/i.test(row.file.name);
            const status =
              row.error ??
              (row.item
                ? "Ready"
                : row.progress >= 99
                  ? "Checking file…"
                  : row.progress > 0
                    ? `Uploading ${row.progress}%`
                    : "Waiting to upload…");
            return (
              <View
                key={row.key}
                style={[
                  styles.card,
                  {
                    backgroundColor: colors.cardSoft,
                    borderColor: colors.border,
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
                  <Label numberOfLines={1}>
                    {row.item?.label ?? row.file.name}
                  </Label>
                  <Label
                    muted
                    style={[
                      styles.status,
                      row.error ? { color: colors.danger } : undefined,
                    ]}
                  >
                    {status}
                  </Label>
                </View>
                {row.item ? (
                  <Check size={16} color={colors.primary} />
                ) : !row.error ? (
                  <ActivityIndicator size="small" color={colors.primary} />
                ) : null}
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
              </View>
            );
          })}
        </ScrollView>
      ) : null}
      <Modal
        visible={!!preview}
        animationType="fade"
        onRequestClose={() => setPreview(undefined)}
      >
        <View style={[styles.preview, { backgroundColor: colors.bg }]}>
          <View style={styles.previewHeader}>
            <Label numberOfLines={1} style={styles.fill}>
              {preview?.file.name}
            </Label>
            <Pressable
              accessibilityRole="button"
              accessibilityLabel="Close file preview"
              onPress={() => setPreview(undefined)}
              style={styles.close}
            >
              <X size={24} color={colors.textPrimary} />
            </Pressable>
          </View>
          {preview ? (
            <Image
              source={{ uri: preview.file.uri }}
              resizeMode="contain"
              style={styles.fullImage}
            />
          ) : null}
        </View>
      </Modal>
    </>
  );
}
const styles = StyleSheet.create({
  fill: { flex: 1, minWidth: 0 },
  list: { maxHeight: 154 },
  content: { gap: 6 },
  card: {
    flexDirection: "row",
    alignItems: "center",
    gap: 8,
    paddingLeft: 10,
    paddingVertical: 4,
    borderWidth: 1,
    borderRadius: radius.md,
  },
  thumbnail: { width: 36, height: 36, borderRadius: 6 },
  status: { fontSize: 11 },
  close: {
    minWidth: 44,
    minHeight: 44,
    alignItems: "center",
    justifyContent: "center",
  },
  preview: {
    flex: 1,
    paddingTop: 60,
    paddingBottom: 32,
    paddingHorizontal: 20,
  },
  previewHeader: { flexDirection: "row", alignItems: "center", gap: 10 },
  fullImage: { flex: 1, width: "100%" },
});
