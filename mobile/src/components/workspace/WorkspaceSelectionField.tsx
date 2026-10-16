import { useEffect, useRef, useState, type ReactNode } from "react";
import {
  FlatList,
  Keyboard,
  KeyboardAvoidingView,
  Modal,
  Platform,
  Pressable,
  StyleSheet,
  TextInput,
  View,
} from "react-native";
import { Check, ChevronDown, Search, X } from "lucide-react-native";
import { SafeAreaProvider, SafeAreaView } from "react-native-safe-area-context";
import { fonts } from "../../theme/designTokens";
import { useTheme } from "../../theme/ThemeContext";
import { ActionButton, Label, Notice } from "./WorkspaceUI";

export type WorkspaceSelectionOption = {
  value: string;
  label: string;
  description?: string;
  disabled?: boolean;
  searchText?: string;
};
type SelectionProps = {
  label: string;
  title?: string;
  options: readonly WorkspaceSelectionOption[];
  value: readonly string[];
  onChange: (values: string[]) => void;
  multiple?: boolean;
  disabled?: boolean;
  readOnly?: boolean;
  maxSelected?: number;
  scopeKey?: string;
  description?: string;
  placeholder?: string;
  summary?: string;
  confirmLabel?: string;
  allowEmpty?: boolean;
  children?: ReactNode;
};
const signature = (value: readonly string[]) =>
  JSON.stringify([...new Set(value)].sort());

/** The enclosing form owns persistence; Apply commits only its local field. */
export function WorkspaceSelectionField(props: SelectionProps) {
  const { colors } = useTheme();
  const [open, setOpen] = useState(false);
  const valueKey = signature(props.value);
  useEffect(() => {
    setOpen(false);
  }, [props.scopeKey, valueKey, props.disabled, props.readOnly]);
  const labels = new Map(
    props.options.map((option) => [option.value, option.label]),
  );
  const selected = [...new Set(props.value)];
  const preview = selected
    .slice(0, 2)
    .map((id) => labels.get(id) ?? id)
    .join(", ");
  const summary =
    props.summary ??
    (selected.length
      ? `${preview}${selected.length > 2 ? ` +${selected.length - 2} more` : ""}`
      : (props.placeholder ?? "Choose"));
  return (
    <View style={styles.stack}>
      <Label>{props.label}</Label>
      <Pressable
        accessibilityRole="button"
        accessibilityLabel={`${props.label}: ${summary}`}
        accessibilityState={{
          disabled: !!props.disabled && !props.readOnly,
          expanded: open,
        }}
        disabled={props.disabled && !props.readOnly}
        style={[
          styles.trigger,
          {
            backgroundColor: colors.cardSoft,
            borderColor: colors.border,
            opacity: props.disabled && !props.readOnly ? 0.5 : 1,
          },
        ]}
        onPress={() => {
          if (props.disabled && !props.readOnly) return;
          Keyboard.dismiss();
          setOpen(true);
        }}
      >
        <Label numberOfLines={2} style={styles.flex}>
          {summary}
        </Label>
        <ChevronDown size={18} color={colors.textSecondary} />
      </Pressable>
      {open && (
        <WorkspaceSelectionSheet {...props} onClose={() => setOpen(false)} />
      )}
    </View>
  );
}

export function WorkspaceSelectionSheet({
  label,
  title = label,
  options,
  value,
  onChange,
  onClose,
  multiple = false,
  disabled = false,
  readOnly = false,
  maxSelected,
  scopeKey,
  description,
  confirmLabel = "Apply",
  allowEmpty = multiple,
  children,
}: SelectionProps & { onClose: () => void }) {
  const { colors } = useTheme();
  const [draft, setDraft] = useState(() => [...new Set(value)]);
  const [query, setQuery] = useState("");
  const [selectedOnly, setSelectedOnly] = useState(false);
  const opening = useRef({
    scopeKey,
    valueKey: signature(value),
    value: [...value],
  }).current;
  const stale =
    opening.scopeKey !== scopeKey || opening.valueKey !== signature(value);
  const applied = useRef(false);
  const blocked = disabled || readOnly || stale;
  const close = () => {
    Keyboard.dismiss();
    onClose();
  };
  useEffect(() => {
    if (stale || (disabled && !readOnly)) onClose();
  }, [stale, disabled, readOnly, onClose]);
  const known = new Map<string, WorkspaceSelectionOption>();
  for (const option of options)
    if (!known.has(option.value)) known.set(option.value, option);
  const rows: WorkspaceSelectionOption[] = [
    ...known.values(),
    ...[...new Set([...opening.value, ...draft])]
      .filter((id) => !known.has(id))
      .map((id) => ({
        value: id,
        label: id,
        description: "Previously selected · unavailable",
      })),
  ];
  const search = query.trim().toLocaleLowerCase();
  const filtered = rows.filter(
    (row) =>
      (!selectedOnly || draft.includes(row.value)) &&
      `${row.label} ${row.description ?? ""} ${row.searchText ?? ""} ${row.value}`
        .toLocaleLowerCase()
        .includes(search),
  );
  const overLimit = maxSelected !== undefined && draft.length > maxSelected;
  const changed = signature(draft) !== opening.valueKey;
  const invalidAddition = draft.some(
    (id) =>
      !opening.value.includes(id) &&
      (!known.has(id) || known.get(id)?.disabled),
  );
  const missingLocked = options.some(
    (row) =>
      row.disabled &&
      opening.value.includes(row.value) &&
      !draft.includes(row.value),
  );
  const canApply =
    !blocked &&
    changed &&
    (allowEmpty || !!draft.length) &&
    (multiple || draft.length === 1 || (allowEmpty && !draft.length)) &&
    !overLimit &&
    !invalidAddition &&
    !missingLocked;
  const lockedIds = new Set(
    options
      .filter((row) => row.disabled && opening.value.includes(row.value))
      .map((row) => row.value),
  );
  const clearable = draft.some((id) => !lockedIds.has(id));
  const searchable = rows.length > 6 || !!query;

  return (
    <Modal
      visible
      animationType="slide"
      presentationStyle="pageSheet"
      onRequestClose={close}
    >
      <SafeAreaProvider>
        <SafeAreaView style={[styles.flex, { backgroundColor: colors.bg }]}>
          <KeyboardAvoidingView
            style={styles.flex}
            behavior={Platform.OS === "ios" ? "padding" : "height"}
          >
            <View style={[styles.header, { borderColor: colors.border }]}>
              <Label accessibilityRole="header" style={styles.heading}>
                {title}
              </Label>
              <Pressable
                accessibilityRole="button"
                accessibilityLabel={`Cancel ${label}`}
                style={styles.iconButton}
                onPress={close}
              >
                <X size={22} color={colors.textSecondary} />
              </Pressable>
            </View>
            {searchable && (
              <View style={styles.toolbar}>
                <View
                  style={[
                    styles.searchBox,
                    {
                      borderColor: colors.border,
                      backgroundColor: colors.card,
                    },
                  ]}
                >
                  <Search size={18} color={colors.textMuted} />
                  <TextInput
                    accessibilityLabel={`Search ${label}`}
                    placeholder="Search options"
                    placeholderTextColor={colors.textMuted}
                    autoCapitalize="none"
                    autoCorrect={false}
                    value={query}
                    onChangeText={setQuery}
                    returnKeyType="search"
                    onSubmitEditing={Keyboard.dismiss}
                    style={[styles.searchInput, { color: colors.textPrimary }]}
                  />
                  {!!query && (
                    <Pressable
                      accessibilityRole="button"
                      accessibilityLabel={`Clear ${label} search`}
                      style={styles.iconButton}
                      onPress={() => setQuery("")}
                    >
                      <X size={18} color={colors.textMuted} />
                    </Pressable>
                  )}
                </View>
                {multiple && (
                  <View accessibilityRole="tablist" style={styles.tabs}>
                    {[false, true].map((only) => (
                      <Pressable
                        key={String(only)}
                        accessibilityRole="tab"
                        accessibilityLabel={
                          only ? "Selected options" : "All options"
                        }
                        accessibilityState={{ selected: only === selectedOnly }}
                        onPress={() => setSelectedOnly(only)}
                        style={[
                          styles.tab,
                          {
                            borderColor:
                              only === selectedOnly
                                ? colors.primaryTone.border
                                : colors.border,
                            backgroundColor:
                              only === selectedOnly
                                ? colors.primaryTone.bg
                                : colors.card,
                          },
                        ]}
                      >
                        <Label>
                          {only
                            ? `Selected (${draft.length})`
                            : `All (${rows.length})`}
                        </Label>
                      </Pressable>
                    ))}
                  </View>
                )}
              </View>
            )}
            <FlatList
              style={styles.flex}
              data={filtered}
              extraData={draft}
              keyExtractor={(item) => item.value}
              keyboardShouldPersistTaps="handled"
              keyboardDismissMode="on-drag"
              contentContainerStyle={styles.list}
              ListHeaderComponent={
                description || children ? (
                  <View style={styles.intro}>
                    {description && <Label muted>{description}</Label>}
                    {children}
                  </View>
                ) : null
              }
              ListEmptyComponent={
                <Label muted>
                  {search
                    ? "No matching options. Try another search."
                    : selectedOnly
                      ? "No options selected. Choose from All."
                      : "No options are available."}
                </Label>
              }
              renderItem={({ item }) => {
                const checked = draft.includes(item.value);
                const capped =
                  multiple &&
                  maxSelected !== undefined &&
                  draft.length >= maxSelected &&
                  !checked;
                const optionBlocked = blocked || !!item.disabled || capped;
                return (
                  <Pressable
                    accessibilityRole={multiple ? "checkbox" : "radio"}
                    accessibilityLabel={item.label}
                    accessibilityHint={item.description}
                    accessibilityState={{ checked, disabled: optionBlocked }}
                    disabled={optionBlocked}
                    onPress={() => {
                      if (optionBlocked || applied.current) return;
                      setDraft((old) => {
                        if (!multiple) return [item.value];
                        if (old.includes(item.value))
                          return old.filter((id) => id !== item.value);
                        if (
                          maxSelected !== undefined &&
                          old.length >= maxSelected
                        )
                          return old;
                        return [...old, item.value];
                      });
                    }}
                    style={[
                      styles.option,
                      {
                        opacity: optionBlocked ? 0.55 : 1,
                        borderColor: checked
                          ? colors.primaryTone.border
                          : colors.border,
                        backgroundColor: checked
                          ? colors.primaryTone.bg
                          : colors.card,
                      },
                    ]}
                  >
                    <View style={styles.flex}>
                      <Label numberOfLines={2}>{item.label}</Label>
                      {item.description && (
                        <Label muted numberOfLines={3}>
                          {item.description}
                        </Label>
                      )}
                    </View>
                    <View
                      style={[
                        styles.check,
                        {
                          borderColor: checked
                            ? colors.primary
                            : colors.textMuted,
                          backgroundColor: checked
                            ? colors.primary
                            : "transparent",
                          borderRadius: multiple ? 4 : 11,
                        },
                      ]}
                    >
                      {checked && <Check size={15} color={colors.onPrimary} />}
                    </View>
                  </Pressable>
                );
              }}
            />
            <View style={[styles.footer, { borderColor: colors.border }]}>
              {overLimit && (
                <Notice
                  message={`Choose up to ${maxSelected} options.`}
                  tone="error"
                />
              )}
              {(invalidAddition || missingLocked) && (
                <Notice
                  message="Available options changed. Review your selection before applying."
                  tone="error"
                />
              )}
              <View style={styles.footerRow}>
                <Label muted style={styles.flex}>
                  {draft.length} selected
                  {maxSelected !== undefined ? ` · Up to ${maxSelected}` : ""}
                </Label>
                {allowEmpty && !readOnly && (
                  <Pressable
                    accessibilityRole="button"
                    accessibilityLabel={
                      lockedIds.size
                        ? "Clear optional choices"
                        : "Clear selection"
                    }
                    disabled={blocked || !clearable}
                    style={styles.clear}
                    onPress={() => {
                      if (!blocked && !applied.current)
                        setDraft((old) =>
                          old.filter((id) => lockedIds.has(id)),
                        );
                    }}
                  >
                    <Label muted>
                      {lockedIds.size
                        ? "Clear optional choices"
                        : "Clear selection"}
                    </Label>
                  </Pressable>
                )}
              </View>
              {readOnly ? (
                <Label muted>These choices are read only.</Label>
              ) : (
                <ActionButton
                  label={confirmLabel}
                  disabled={!canApply}
                  onPress={() => {
                    if (!canApply || applied.current) return;
                    applied.current = true;
                    onChange([...draft]);
                    close();
                  }}
                />
              )}
            </View>
          </KeyboardAvoidingView>
        </SafeAreaView>
      </SafeAreaProvider>
    </Modal>
  );
}

const styles = StyleSheet.create({
  flex: { flex: 1, minWidth: 0 },
  stack: { gap: 8 },
  trigger: {
    flexDirection: "row",
    alignItems: "center",
    gap: 12,
    minHeight: 44,
    borderWidth: 1,
    borderRadius: 8,
    padding: 12,
  },
  header: {
    flexDirection: "row",
    alignItems: "center",
    gap: 12,
    paddingHorizontal: 20,
    paddingVertical: 8,
    borderBottomWidth: 1,
  },
  heading: {
    flex: 1,
    fontFamily: fonts.displaySemi,
    fontSize: 18,
    lineHeight: 25,
  },
  iconButton: {
    minWidth: 44,
    minHeight: 44,
    justifyContent: "center",
    alignItems: "center",
  },
  toolbar: { paddingHorizontal: 20, paddingTop: 16, gap: 12 },
  searchBox: {
    flexDirection: "row",
    alignItems: "center",
    gap: 8,
    borderWidth: 1,
    borderRadius: 8,
    paddingLeft: 12,
  },
  searchInput: {
    flex: 1,
    minWidth: 0,
    minHeight: 44,
    paddingVertical: 8,
    paddingRight: 8,
    fontFamily: fonts.bodyRegular,
    fontSize: 14,
  },
  tabs: { flexDirection: "row", gap: 8 },
  tab: {
    minHeight: 44,
    paddingHorizontal: 14,
    justifyContent: "center",
    borderWidth: 1,
    borderRadius: 8,
  },
  list: { padding: 20, gap: 10, flexGrow: 1 },
  intro: { gap: 12, paddingBottom: 8 },
  option: {
    flexDirection: "row",
    alignItems: "center",
    gap: 12,
    minHeight: 52,
    borderWidth: 1,
    borderRadius: 12,
    padding: 14,
  },
  check: {
    width: 22,
    height: 22,
    borderWidth: 1.5,
    justifyContent: "center",
    alignItems: "center",
  },
  footer: {
    paddingHorizontal: 20,
    paddingTop: 8,
    paddingBottom: 12,
    gap: 8,
    borderTopWidth: 1,
  },
  footerRow: { flexDirection: "row", alignItems: "center", gap: 8 },
  clear: { minHeight: 44, justifyContent: "center", paddingLeft: 8 },
});
