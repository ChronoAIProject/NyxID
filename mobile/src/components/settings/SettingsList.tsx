import { useLayoutEffect } from "react";
import type { ReactElement, ReactNode, Ref } from "react";
import {
  ActivityIndicator,
  Pressable,
  RefreshControl,
  ScrollView,
  StyleSheet,
  TextInput,
  View,
  useWindowDimensions,
  type TextInputProps,
  type AccessibilityRole,
  type AccessibilityState,
  type LayoutChangeEvent,
} from "react-native";
import { Check, ChevronRight, type LucideIcon } from "lucide-react-native";
import { DynamicTypeText as Text } from "../DynamicTypeText";
import { ScreenContainer } from "../ScreenContainer";
import { useTheme } from "../../theme/ThemeContext";
import { fonts, radius, spacing, typeScale } from "../../theme/designTokens";

// Leading inset of the hairline between rows: past the icon tile, as on iOS.
const ICON_TILE = 30;

/** Rows wrap to stacked label/value when scaled text leaves too little width. */
export function useSettingsStacked() {
  const { width, fontScale } = useWindowDimensions();
  return width / fontScale < 340;
}

/** Scrollable page frame shared by Settings and its sub-pages. */
export function SettingsPage({
  children,
  scrollRef,
  refreshing,
  onRefresh,
}: {
  children: ReactNode;
  scrollRef?: Ref<ScrollView>;
  /** Pull to refresh, for pages that show server data. */
  refreshing?: boolean;
  onRefresh?: () => void;
}) {
  return (
    <ScreenContainer>
      <ScrollView
        ref={scrollRef}
        refreshControl={
          onRefresh ? (
            <RefreshControl refreshing={!!refreshing} onRefresh={onRefresh} />
          ) : undefined
        }
        style={styles.page}
        contentContainerStyle={styles.pageContent}
        showsVerticalScrollIndicator={false}
      >
        {children}
      </ScrollView>
    </ScreenContainer>
  );
}

/** An inset grouped list: optional small title, rows in one card, optional footer note. */
export function SettingsGroup({
  title,
  footer,
  inset = true,
  onLayout,
  children,
}: {
  title?: string;
  footer?: ReactNode;
  /** Rows have leading icons, so separators start after the icon tile. */
  inset?: boolean;
  onLayout?: (event: LayoutChangeEvent) => void;
  children: ReactNode;
}) {
  const { colors } = useTheme();
  const rows = [children]
    .flat(Infinity as 1)
    .filter(
      (child): child is ReactElement =>
        !!child && typeof child === "object" && "props" in child,
    );
  return (
    <View onLayout={onLayout} style={styles.group}>
      {title ? (
        <Text
          accessibilityRole="header"
          style={[styles.groupTitle, { color: colors.textMuted }]}
        >
          {title.toUpperCase()}
        </Text>
      ) : null}
      <View style={[styles.card, { backgroundColor: colors.card }]}>
        {rows.map((row, index) => (
          <View key={row.key ?? index}>
            {index > 0 ? (
              <View
                style={[
                  styles.separator,
                  {
                    backgroundColor: colors.borderSoft,
                    marginLeft: inset
                      ? spacing.xxl + ICON_TILE + spacing.lg
                      : spacing.xxl,
                  },
                ]}
              />
            ) : null}
            {row}
          </View>
        ))}
      </View>
      {typeof footer === "string" ? (
        <Text style={[styles.footer, { color: colors.textMuted }]}>
          {footer}
        </Text>
      ) : (
        (footer ?? null)
      )}
    </View>
  );
}

/**
 * One settings row: duotone icon tile, label (and optional detail line), then a
 * value, an inline control, or a chevron when it opens a sub-page.
 */
export function SettingsRow({
  icon: Icon,
  leading,
  label,
  detail,
  value,
  accessory,
  onPress,
  chevron: chevronProp,
  destructive = false,
  tone = "default",
  mono = false,
  selectable = false,
  centered = false,
  disabled = false,
  accessibilityRole,
  accessibilityState,
  accessibilityHint,
  accessibilityLabel,
  valueTone = "default",
}: {
  icon?: LucideIcon;
  /** Custom leading content (e.g. a colour swatch) in the icon slot. */
  leading?: ReactNode;
  label: string;
  detail?: string;
  value?: string;
  accessory?: ReactNode;
  onPress?: () => void;
  chevron?: boolean;
  destructive?: boolean;
  /** iOS action rows: tinted ("action") or red ("danger") text, no fill. */
  tone?: "default" | "action" | "danger";
  /** Monospaced label for codes and keys. */
  mono?: boolean;
  selectable?: boolean;
  /** Centred action rows (Sign Out, Delete Account) carry no icon or chevron. */
  centered?: boolean;
  disabled?: boolean;
  accessibilityRole?: AccessibilityRole;
  accessibilityState?: AccessibilityState;
  accessibilityHint?: string;
  /** Spoken label when it must carry more than the visible text (exact amounts). */
  accessibilityLabel?: string;
  valueTone?: "default" | "danger";
}) {
  const { colors } = useTheme();
  const stacked = useSettingsStacked();
  // Rows that open a sub-page get a chevron; centred actions never do.
  const chevron =
    !centered &&
    tone === "default" &&
    (chevronProp ?? (!!onPress && !accessory));
  // Destructive rows are a solid red button; disabled ones fall back to the
  // neutral card so the muted label stays readable.
  const filled = destructive && !disabled;
  const labelColor = filled
    ? colors.onPrimary
    : tone === "action"
      ? colors.primary
      : tone === "danger"
        ? colors.danger
        : colors.textPrimary;
  const content = (
    <>
      {(Icon || leading) && !centered ? (
        // Bare glyph in a fixed slot so separators align past every icon.
        <View style={styles.tile}>
          {Icon ? (
            <Icon size={17} color={colors.textPrimary} strokeWidth={2} />
          ) : (
            leading
          )}
        </View>
      ) : null}
      <View
        style={[
          styles.body,
          stacked && !centered ? styles.bodyStacked : styles.bodyInline,
          centered && styles.bodyCentered,
        ]}
      >
        <View style={styles.labels}>
          <Text
            selectable={selectable}
            style={[
              styles.label,
              mono && { fontFamily: fonts.mono },
              { color: disabled ? colors.textMuted : labelColor },
              centered && styles.centeredLabel,
            ]}
          >
            {label}
          </Text>
          {detail ? (
            <Text style={[styles.detail, { color: colors.textMuted }]}>
              {detail}
            </Text>
          ) : null}
        </View>
        {value ? (
          <View style={styles.valueWrap}>
            <Text
              style={[
                styles.value,
                {
                  color:
                    valueTone === "danger" ? colors.danger : colors.textSecondary,
                  textAlign: stacked ? "left" : "right",
                },
              ]}
            >
              {value}
            </Text>
          </View>
        ) : null}
        {accessory}
      </View>
      {chevron ? <ChevronRight size={18} color={colors.textMuted} /> : null}
    </>
  );
  const fill = filled ? { backgroundColor: colors.dangerSolid } : null;
  if (!onPress)
    return (
      <View
        accessible={accessibilityLabel ? true : undefined}
        accessibilityLabel={accessibilityLabel}
        style={[styles.row, fill]}
      >
        {content}
      </View>
    );
  return (
    <Pressable
      accessibilityRole={accessibilityRole ?? "button"}
      accessibilityLabel={
        accessibilityLabel ?? (value ? `${label}, ${value}` : label)
      }
      accessibilityHint={
        accessibilityHint ?? (chevron ? `Opens ${label}` : undefined)
      }
      accessibilityState={{ disabled, ...accessibilityState }}
      disabled={disabled}
      onPress={onPress}
      style={({ pressed }) => [
        styles.row,
        fill,
        pressed &&
          !disabled && {
            backgroundColor: filled
              ? colors.dangerSolidPressed
              : colors.navActive,
          },
      ]}
    >
      {content}
    </Pressable>
  );
}

/**
 * iOS text entry inside a group: the field fills the row, with no box of its
 * own. Used on the single-purpose edit pages that rows open.
 */
export function SettingsTextField(props: TextInputProps) {
  const { colors } = useTheme();
  return (
    <View style={styles.row}>
      <TextInput
        clearButtonMode="while-editing"
        placeholderTextColor={colors.textMuted}
        {...props}
        style={[styles.field, { color: colors.textPrimary }, props.style]}
      />
    </View>
  );
}

/** One option in a choice list; the selected option carries a checkmark. */
export function SettingsCheckRow({
  label,
  detail,
  selected,
  onPress,
  leading,
  disabled = false,
}: {
  leading?: ReactNode;
  label: string;
  detail?: string;
  selected: boolean;
  onPress: () => void;
  disabled?: boolean;
}) {
  const { colors } = useTheme();
  return (
    <SettingsRow
      leading={leading}
      label={label}
      detail={detail}
      onPress={onPress}
      disabled={disabled}
      chevron={false}
      accessibilityRole="radio"
      accessibilityState={{ selected, checked: selected }}
      accessibilityHint={selected ? "Selected" : undefined}
      accessory={
        selected ? (
          <Check size={18} color={colors.primary} strokeWidth={2.5} />
        ) : (
          <View style={{ width: 18 }} />
        )
      }
    />
  );
}

/**
 * Edit pages put their commit action where iOS does: top right of the bar.
 * It replaces the sheet's close button while the page is shown.
 */
export function useSettingsHeaderAction(
  navigation: { setOptions: (options: object) => void },
  {
    label = "Save",
    enabled,
    busy = false,
    onPress,
  }: { label?: string; enabled: boolean; busy?: boolean; onPress: () => void },
) {
  const { colors } = useTheme();
  useLayoutEffect(() => {
    navigation.setOptions({
      headerRight: () =>
        busy ? (
          <ActivityIndicator color={colors.primary} />
        ) : (
          <Pressable
            accessibilityRole="button"
            accessibilityLabel={label}
            accessibilityState={{ disabled: !enabled }}
            disabled={!enabled}
            hitSlop={12}
            onPress={onPress}
            style={styles.headerAction}
          >
            <Text
              style={[
                styles.headerActionLabel,
                { color: enabled ? colors.primary : colors.textMuted },
              ]}
            >
              {label}
            </Text>
          </Pressable>
        ),
    });
  }, [navigation, label, enabled, busy, onPress, colors]);
}

const styles = StyleSheet.create({
  page: { flex: 1 },
  field: { flex: 1, minHeight: 28, ...typeScale.label },
  headerAction: { minHeight: 44, justifyContent: "center", paddingHorizontal: 4 },
  headerActionLabel: { ...typeScale.label, fontFamily: fonts.bodySemi, fontWeight: "600" },
  pageContent: {
    paddingTop: spacing.sm,
    paddingHorizontal: spacing.xxl,
    paddingBottom: spacing.huge,
    gap: spacing.huge,
  },
  group: { gap: spacing.sm },
  groupTitle: {
    ...typeScale.small,
    letterSpacing: 0.6,
    paddingHorizontal: spacing.xxl,
  },
  card: { borderRadius: radius.lg, overflow: "hidden" },
  separator: { height: StyleSheet.hairlineWidth },
  footer: {
    ...typeScale.small,
    paddingHorizontal: spacing.xxl,
  },
  row: {
    flexDirection: "row",
    alignItems: "center",
    gap: spacing.lg,
    minHeight: 52,
    paddingHorizontal: spacing.xxl,
    paddingVertical: spacing.md,
  },
  tile: {
    width: ICON_TILE,
    height: ICON_TILE,
    alignItems: "center",
    justifyContent: "center",
  },
  body: {
    flex: 1,
    minWidth: 0,
    maxWidth: "100%",
    justifyContent: "space-between",
    gap: spacing.sm,
  },
  bodyInline: { flexDirection: "row", alignItems: "center" },
  bodyStacked: { flexDirection: "column", alignItems: "flex-start" },
  bodyCentered: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "center",
  },
  labels: { flexShrink: 1, minWidth: 0, maxWidth: "100%", gap: 2 },
  label: { ...typeScale.label },
  centeredLabel: { textAlign: "center" },
  detail: { ...typeScale.small },
  valueWrap: { minWidth: 0, maxWidth: "100%", flexShrink: 1 },
  value: { ...typeScale.description },
});
