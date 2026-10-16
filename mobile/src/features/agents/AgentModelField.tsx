import { useEffect, useMemo, useState } from "react";
import {
  FlatList,
  Keyboard,
  KeyboardAvoidingView,
  Modal,
  Platform,
  Pressable,
  StyleSheet,
  View,
} from "react-native";
import { Check, ChevronDown } from "lucide-react-native";
import { SafeAreaProvider, SafeAreaView } from "react-native-safe-area-context";
import {
  ActionButton,
  Field,
  Label,
} from "../../components/workspace/WorkspaceUI";
import { fonts, TOUCH_TARGET } from "../../theme/designTokens";
import { useTheme } from "../../theme/ThemeContext";

type Model = { id: string; label: string; available: boolean };

/** A draft choice updates the profile only after Apply; Save profile persists it. */
export function AgentModelField({
  value,
  models,
  disabled,
  onChange,
}: {
  value: string | undefined;
  models: Model[];
  disabled: boolean;
  onChange: (value: string | undefined) => void;
}) {
  const { colors } = useTheme();
  const [open, setOpen] = useState(false);
  const [draft, setDraft] = useState(value);
  const [query, setQuery] = useState("");
  const current = models.find((model) => model.id === value);
  const currentLabel = current?.label ?? value ?? "Workspace default";

  useEffect(() => {
    setOpen(false);
    setDraft(value);
    setQuery("");
  }, [value, disabled]);

  const rows = useMemo(() => {
    const seen = new Set<string>();
    const options = models.filter((model) => {
      if (seen.has(model.id)) return false;
      seen.add(model.id);
      return true;
    });
    if (value && !seen.has(value))
      options.unshift({ id: value, label: value, available: false });
    const search = query.trim().toLocaleLowerCase();
    return search
      ? options.filter((model) =>
          `${model.label} ${model.id}`.toLocaleLowerCase().includes(search),
        )
      : options;
  }, [models, value, query]);
  const canApply =
    !disabled &&
    draft !== value &&
    models.some((model) => model.id === draft && model.available);

  const close = () => {
    Keyboard.dismiss();
    setOpen(false);
    setDraft(value);
    setQuery("");
  };

  return (
    <View style={styles.stack}>
      <Label>Model for new threads</Label>
      <Pressable
        accessibilityRole="button"
        accessibilityLabel={`Model for new threads: ${currentLabel}`}
        accessibilityHint="Choose a model. Save the profile to persist your choice."
        accessibilityState={{ disabled, expanded: open }}
        disabled={disabled}
        onPress={() => {
          if (disabled) return;
          Keyboard.dismiss();
          setDraft(value);
          setQuery("");
          setOpen(true);
        }}
        style={[
          styles.trigger,
          {
            backgroundColor: colors.cardSoft,
            borderColor: colors.border,
            opacity: disabled ? 0.5 : 1,
          },
        ]}
      >
        <View style={styles.flex}>
          <Label numberOfLines={2}>{currentLabel}</Label>
          {value && current?.available !== true && (
            <Label muted>Currently selected · unavailable</Label>
          )}
        </View>
        <ChevronDown size={18} color={colors.textSecondary} />
      </Pressable>
      <Modal
        visible={open}
        animationType="slide"
        presentationStyle="pageSheet"
        onRequestClose={close}
        onDismiss={close}
      >
        <SafeAreaProvider>
          <SafeAreaView
            edges={["top", "bottom", "left", "right"]}
            style={[styles.flex, { backgroundColor: colors.bg }]}
          >
            <KeyboardAvoidingView
              behavior={Platform.OS === "ios" ? "padding" : undefined}
              style={styles.flex}
            >
              <View style={[styles.header, { borderColor: colors.border }]}>
                <Label accessibilityRole="header" style={styles.title}>
                  Choose a model
                </Label>
                <Pressable
                  accessibilityRole="button"
                  accessibilityLabel="Cancel model selection"
                  onPress={close}
                  style={styles.cancel}
                >
                  <Label style={{ color: colors.primary }}>Cancel</Label>
                </Pressable>
              </View>
              <View style={styles.search}>
                <Label muted>
                  Applies to new threads after you save the profile.
                </Label>
                <Field
                  label="Search models"
                  value={query}
                  onChangeText={setQuery}
                  autoCorrect={false}
                  clearButtonMode="while-editing"
                  returnKeyType="search"
                />
                <Label muted>Current: {currentLabel}</Label>
              </View>
              <FlatList
                style={styles.flex}
                data={rows}
                keyExtractor={(model) => model.id}
                keyboardShouldPersistTaps="handled"
                keyboardDismissMode="on-drag"
                contentContainerStyle={styles.list}
                ListEmptyComponent={
                  <Label muted>
                    {query.trim()
                      ? "No matching models. Try another search."
                      : "No models are available. Your current choice is preserved."}
                  </Label>
                }
                renderItem={({ item }) => {
                  const checked = draft === item.id;
                  const blocked = disabled || !item.available;
                  return (
                    <Pressable
                      accessibilityRole="radio"
                      accessibilityLabel={`${item.label}${item.available ? "" : " · unavailable"}`}
                      accessibilityState={{ checked, disabled: blocked }}
                      disabled={blocked}
                      onPress={() => {
                        if (!blocked) setDraft(item.id);
                      }}
                      style={[
                        styles.option,
                        {
                          backgroundColor: checked
                            ? colors.primaryTone.bg
                            : colors.card,
                          borderColor: checked ? colors.primary : colors.border,
                          opacity: blocked ? 0.6 : 1,
                        },
                      ]}
                    >
                      <View style={styles.flex}>
                        <Label>{item.label}</Label>
                        {item.label !== item.id && (
                          <Label muted mono style={styles.modelId}>
                            {item.id}
                          </Label>
                        )}
                        {!item.available && <Label muted>Unavailable</Label>}
                      </View>
                      {checked && <Check size={18} color={colors.primary} />}
                    </Pressable>
                  );
                }}
              />
              <View style={[styles.footer, { borderColor: colors.border }]}>
                <ActionButton
                  label="Apply model"
                  disabled={!canApply}
                  onPress={() => {
                    if (!canApply) return;
                    onChange(draft);
                    close();
                  }}
                />
              </View>
            </KeyboardAvoidingView>
          </SafeAreaView>
        </SafeAreaProvider>
      </Modal>
    </View>
  );
}

const styles = StyleSheet.create({
  flex: { flex: 1 },
  stack: { gap: 8 },
  trigger: {
    flexDirection: "row",
    alignItems: "center",
    gap: 12,
    minHeight: TOUCH_TARGET,
    borderWidth: 1,
    borderRadius: 8,
    padding: 12,
  },
  header: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
    gap: 12,
    borderBottomWidth: 1,
    paddingHorizontal: 20,
    paddingVertical: 8,
  },
  title: { fontFamily: fonts.displaySemi, fontSize: 18, flex: 1 },
  cancel: {
    minHeight: TOUCH_TARGET,
    justifyContent: "center",
    paddingLeft: 12,
  },
  search: { gap: 12, padding: 20, paddingBottom: 12 },
  list: { gap: 8, padding: 20, paddingTop: 0 },
  option: {
    flexDirection: "row",
    alignItems: "center",
    gap: 12,
    minHeight: TOUCH_TARGET,
    padding: 12,
    borderRadius: 8,
    borderWidth: 1,
  },
  modelId: { fontSize: 11 },
  footer: { padding: 20, borderTopWidth: 1 },
});
