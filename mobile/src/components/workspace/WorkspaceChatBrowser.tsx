import { useReducedMotion } from "../../lib/accessibility/useReducedMotion";
import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type ReactNode,
} from "react";
import {
  FlatList,
  Keyboard,
  KeyboardAvoidingView,
  Modal,
  Platform,
  Pressable,
  StyleSheet,
  Text,
  TextInput,
  View,
} from "react-native";
import { ChevronLeft, Search, X } from "lucide-react-native";
import { SafeAreaProvider, SafeAreaView } from "react-native-safe-area-context";
import { fonts } from "../../theme/designTokens";
import { useTheme } from "../../theme/ThemeContext";
import { searchDrawerItems } from "./workspaceChatListModel";

export function WorkspaceChatBrowser<T extends { id: string }>({
  title,
  scopeKey,
  description,
  items,
  searchText,
  renderItem,
  onClose,
  onBack,
  primaryAction,
  visible = true,
  onDismiss,
  footer,
  emptyText = "Nothing here yet.",
}: {
  title: string;
  scopeKey: string;
  description?: string;
  items: readonly T[];
  searchText: (item: T) => string;
  renderItem: (item: T) => ReactNode;
  onClose: () => void;
  onBack?: () => void;
  visible?: boolean;
  onDismiss?: () => void;
  primaryAction?: {
    label: string;
    accessibilityLabel: string;
    onPress: () => void;
  };
  footer?: ReactNode;
  emptyText?: string;
}) {
  const { colors } = useTheme();
  const reducedMotion = useReducedMotion();
  const [query, setQuery] = useState("");
  const visibleRef = useRef(visible);
  visibleRef.current = visible;
  const onDismissRef = useRef(onDismiss);
  onDismissRef.current = onDismiss;
  const mountedRef = useRef(true);
  const dismissalPending = useRef(false);
  const previousVisibleRef = useRef(visible);
  if (previousVisibleRef.current !== visible) {
    dismissalPending.current = !visible;
    previousVisibleRef.current = visible;
  }
  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
    };
  }, []);
  const notifyDismissed = useCallback(() => {
    if (!mountedRef.current || visibleRef.current || !dismissalPending.current)
      return;
    dismissalPending.current = false;
    onDismissRef.current?.();
  }, []);
  useEffect(() => {
    // Android removes its native Modal in this commit and has no onDismiss.
    if (!visible && Platform.OS !== "ios") notifyDismissed();
  }, [visible, notifyDismissed]);
  // Switching between agent/group lists and chat history starts a fresh search.
  useEffect(() => setQuery(""), [scopeKey]);
  const filtered = searchDrawerItems(items, query, searchText);
  const searching = !!query.trim();
  const close = () => {
    Keyboard.dismiss();
    onClose();
  };

  return (
    <Modal
      visible={visible}
      animationType={reducedMotion ? "none" : "slide"}
      presentationStyle="pageSheet"
      onRequestClose={close}
      onDismiss={Platform.OS === "ios" ? notifyDismissed : undefined}
    >
      <SafeAreaProvider style={styles.fill}>
        <SafeAreaView style={[styles.fill, { backgroundColor: colors.bg }]}>
          <KeyboardAvoidingView
            style={styles.fill}
            behavior={Platform.OS === "ios" ? "padding" : "height"}
          >
            <View style={[styles.header, { borderColor: colors.border }]}>
              {onBack ? (
                <Pressable
                  accessibilityRole="button"
                  accessibilityLabel="Back to previous list"
                  onPress={() => {
                    Keyboard.dismiss();
                    onBack();
                  }}
                  style={styles.iconButton}
                >
                  <ChevronLeft size={22} color={colors.textSecondary} />
                </Pressable>
              ) : null}
              <Text
                accessibilityRole="header"
                numberOfLines={2}
                style={[styles.heading, { color: colors.textPrimary }]}
              >
                {title}
              </Text>
              <Pressable
                accessibilityRole="button"
                accessibilityLabel={`Close ${title}`}
                onPress={close}
                style={styles.iconButton}
              >
                <X size={22} color={colors.textSecondary} />
              </Pressable>
            </View>
            <View style={styles.toolbar}>
              {description ? (
                <Text
                  style={[styles.description, { color: colors.textSecondary }]}
                >
                  {description}
                </Text>
              ) : null}
              {primaryAction ? (
                <Pressable
                  accessibilityRole="button"
                  accessibilityLabel={primaryAction.accessibilityLabel}
                  onPress={() => {
                    Keyboard.dismiss();
                    primaryAction.onPress();
                  }}
                  style={[
                    styles.primaryAction,
                    { backgroundColor: colors.primary },
                  ]}
                >
                  <Text
                    style={[
                      styles.primaryActionLabel,
                      { color: colors.onPrimary },
                    ]}
                  >
                    {primaryAction.label}
                  </Text>
                </Pressable>
              ) : null}
              {items.length > 0 || searching ? (
                <>
                  <View
                    style={[
                      styles.searchBox,
                      {
                        backgroundColor: colors.card,
                        borderColor: colors.border,
                      },
                    ]}
                  >
                    <Search size={18} color={colors.textMuted} />
                    <TextInput
                      accessibilityLabel={`Search ${title}`}
                      placeholder={`Search ${title.toLowerCase()}`}
                      placeholderTextColor={colors.textMuted}
                      autoCapitalize="none"
                      autoCorrect={false}
                      value={query}
                      onChangeText={setQuery}
                      returnKeyType="search"
                      onSubmitEditing={Keyboard.dismiss}
                      style={[
                        styles.searchInput,
                        { color: colors.textPrimary },
                      ]}
                    />
                    {query ? (
                      <Pressable
                        accessibilityRole="button"
                        accessibilityLabel={`Clear ${title} search`}
                        onPress={() => setQuery("")}
                        style={styles.iconButton}
                      >
                        <X size={18} color={colors.textMuted} />
                      </Pressable>
                    ) : null}
                  </View>
                  <Text
                    accessibilityLiveRegion="polite"
                    style={[styles.count, { color: colors.textMuted }]}
                  >
                    {filtered.length}{" "}
                    {filtered.length === 1 ? "result" : "results"}
                  </Text>
                </>
              ) : null}
            </View>
            <FlatList
              key={scopeKey}
              style={styles.fill}
              data={filtered}
              keyExtractor={(item) => item.id}
              renderItem={({ item }) => <View>{renderItem(item)}</View>}
              keyboardShouldPersistTaps="handled"
              keyboardDismissMode="on-drag"
              contentContainerStyle={styles.list}
              ListEmptyComponent={
                <View style={styles.empty}>
                  <Text
                    style={[styles.emptyTitle, { color: colors.textPrimary }]}
                  >
                    {searching ? "No matches" : emptyText}
                  </Text>
                  {searching ? (
                    <Text
                      style={[
                        styles.description,
                        { color: colors.textSecondary },
                      ]}
                    >
                      Try another search.
                    </Text>
                  ) : null}
                </View>
              }
              ListFooterComponent={
                footer ? <View style={styles.footer}>{footer}</View> : null
              }
            />
          </KeyboardAvoidingView>
        </SafeAreaView>
      </SafeAreaProvider>
    </Modal>
  );
}

const styles = StyleSheet.create({
  fill: { flex: 1 },
  header: {
    flexDirection: "row",
    alignItems: "center",
    gap: 8,
    minHeight: 64,
    paddingHorizontal: 16,
    paddingVertical: 8,
    borderBottomWidth: 1,
  },
  heading: {
    flex: 1,
    fontFamily: fonts.bodySemi,
    fontSize: 19,
    lineHeight: 27,
  },
  iconButton: {
    width: 44,
    height: 44,
    alignItems: "center",
    justifyContent: "center",
  },
  toolbar: { paddingHorizontal: 20, paddingTop: 16, gap: 12 },
  primaryAction: {
    minHeight: 44,
    paddingHorizontal: 16,
    paddingVertical: 12,
    borderRadius: 8,
    alignItems: "center",
    justifyContent: "center",
  },
  primaryActionLabel: {
    fontFamily: fonts.bodySemi,
    fontSize: 14,
    lineHeight: 20,
  },
  description: {
    fontFamily: fonts.bodyRegular,
    fontSize: 13,
    lineHeight: 20,
  },
  searchBox: {
    flexDirection: "row",
    alignItems: "center",
    gap: 10,
    paddingLeft: 14,
    paddingRight: 4,
    borderWidth: 1,
    borderRadius: 12,
    minHeight: 48,
  },
  searchInput: {
    flex: 1,
    minWidth: 0,
    minHeight: 44,
    paddingVertical: 10,
    fontFamily: fonts.bodyRegular,
    fontSize: 16,
  },
  count: { fontFamily: fonts.body, fontSize: 12, lineHeight: 18 },
  list: { paddingHorizontal: 16, paddingTop: 8, paddingBottom: 24, gap: 4 },
  empty: { paddingHorizontal: 12, paddingVertical: 32, gap: 8 },
  emptyTitle: { fontFamily: fonts.bodySemi, fontSize: 15, lineHeight: 22 },
  footer: { paddingHorizontal: 4, paddingTop: 12 },
});
