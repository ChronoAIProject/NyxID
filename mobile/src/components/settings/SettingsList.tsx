import type { ReactElement, ReactNode, Ref } from "react";
import {
  Pressable,
  ScrollView,
  StyleSheet,
  View,
  useWindowDimensions,
  type AccessibilityRole,
  type AccessibilityState,
  type LayoutChangeEvent,
} from "react-native";
import { ChevronRight, type LucideIcon } from "lucide-react-native";
import { DynamicTypeText as Text } from "../DynamicTypeText";
import { ScreenContainer } from "../ScreenContainer";
import { useTheme } from "../../theme/ThemeContext";
import { radius, spacing, typeScale } from "../../theme/designTokens";

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
}: {
  children: ReactNode;
  scrollRef?: Ref<ScrollView>;
}) {
  return (
    <ScreenContainer>
      <ScrollView
        ref={scrollRef}
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
  label,
  detail,
  value,
  accessory,
  onPress,
  chevron: chevronProp,
  destructive = false,
  centered = false,
  disabled = false,
  accessibilityRole,
  accessibilityState,
  accessibilityHint,
}: {
  icon?: LucideIcon;
  label: string;
  detail?: string;
  value?: string;
  accessory?: ReactNode;
  onPress?: () => void;
  chevron?: boolean;
  destructive?: boolean;
  /** Centred action rows (Sign Out, Delete Account) carry no icon or chevron. */
  centered?: boolean;
  disabled?: boolean;
  accessibilityRole?: AccessibilityRole;
  accessibilityState?: AccessibilityState;
  accessibilityHint?: string;
}) {
  const { colors } = useTheme();
  const stacked = useSettingsStacked();
  // Rows that open a sub-page get a chevron; centred actions never do.
  const chevron = !centered && (chevronProp ?? (!!onPress && !accessory));
  // Destructive rows are a solid red button; disabled ones fall back to the
  // neutral card so the muted label stays readable.
  const filled = destructive && !disabled;
  const labelColor = filled ? colors.onPrimary : colors.textPrimary;
  const content = (
    <>
      {Icon && !centered ? (
        // Bare glyph in a fixed slot so separators align past every icon.
        <View style={styles.tile}>
          <Icon size={17} color={colors.textPrimary} strokeWidth={2} />
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
            style={[
              styles.label,
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
                  color: colors.textSecondary,
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
  if (!onPress) return <View style={[styles.row, fill]}>{content}</View>;
  return (
    <Pressable
      accessibilityRole={accessibilityRole ?? "button"}
      accessibilityLabel={value ? `${label}, ${value}` : label}
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

const styles = StyleSheet.create({
  page: { flex: 1 },
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
