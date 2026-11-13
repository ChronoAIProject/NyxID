import { useAppLanguage } from "../../i18n/LanguageProvider";
import { useEffect, useLayoutEffect, useRef, useState } from "react";
import type { ReactElement, ReactNode, Ref } from "react";
import {
  ActivityIndicator,
  Modal,
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
import {
  Check,
  ChevronRight,
  ChevronsUpDown,
  type LucideIcon,
} from "lucide-react-native";
import { DynamicTypeText as Text } from "../DynamicTypeText";
import { ScreenContainer } from "../ScreenContainer";
import { PageScrollView } from "../workspace/ScrollEdgeFades";
import { useTheme } from "../../theme/ThemeContext";
import { fonts, radius, spacing, typeScale } from "../../theme/designTokens";

// Leading inset of the hairline between rows: past the icon tile, as on iOS.
const ICON_TILE = 30;

/** Rows wrap to stacked label/value when scaled text leaves too little width. */
export function useSettingsStacked(availableWidth?: number) {
  const { width, fontScale } = useWindowDimensions();
  return (availableWidth ?? width) / fontScale < 340;
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
  const { colors } = useTheme();
  return (
    <ScreenContainer>
      <PageScrollView
        scrollRef={scrollRef}
        fadeColor={colors.bg}
        refreshControl={
          onRefresh ? (
            <RefreshControl refreshing={!!refreshing} onRefresh={onRefresh} />
          ) : undefined
        }
        style={styles.page}
        contentContainerStyle={styles.pageContent}
        showsVerticalScrollIndicator={false}
        keyboardShouldPersistTaps="handled"
        keyboardDismissMode="interactive"
      >
        {children}
      </PageScrollView>
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
      <View style={[styles.card, { backgroundColor: colors.groupedFill }]}>
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
  badge,
  badgeTone = "default",
  detail,
  detailLines,
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
  /** A short tag beside the label (e.g. "OAuth"); it never wraps. */
  badge?: string;
  badgeTone?: "default" | "warning" | "danger";
  detail?: string;
  /** Clamp a long detail line (catalog descriptions) to this many lines. */
  detailLines?: number;
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
  const { t } = useAppLanguage();
  const { colors } = useTheme();
  const [rowWidth, setRowWidth] = useState<number>();
  const stacked = useSettingsStacked(rowWidth);
  const measureRow = ({ nativeEvent: { layout } }: LayoutChangeEvent) => {
    if (layout.width > 0) setRowWidth(layout.width);
  };
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
          <View style={styles.labelLine}>
            <Text
              selectable={selectable}
              style={[
                styles.label,
                styles.labelText,
                mono && { fontFamily: fonts.mono },
                { color: disabled ? colors.textMuted : labelColor },
                centered && styles.centeredLabel,
              ]}
            >
              {label}
            </Text>
            {badge ? (
              <View
                style={[
                  styles.badge,
                  {
                    backgroundColor:
                      badgeTone === "danger"
                        ? colors.dangerSoftBg
                        : badgeTone === "warning"
                          ? colors.warningSoft
                          : colors.segmentTrack,
                  },
                ]}
              >
                <Text
                  numberOfLines={1}
                  maxFontSizeMultiplier={1.8}
                  style={[
                    styles.badgeText,
                    {
                      color:
                        badgeTone === "danger"
                          ? colors.danger
                          : badgeTone === "warning"
                            ? colors.warning
                            : colors.textSecondary,
                    },
                  ]}
                >
                  {badge}
                </Text>
              </View>
            ) : null}
          </View>
          {detail ? (
            <Text
              numberOfLines={detailLines}
              style={[styles.detail, { color: colors.textMuted }]}
            >
              {detail}
            </Text>
          ) : null}
        </View>
        {value || accessory ? (
          // Value and control stay together at the trailing edge; a short
          // value is never squeezed into wrapping by a long label.
          <View
            style={[
              styles.trailing,
              stacked && !centered && styles.trailingStacked,
            ]}
          >
            {value ? (
              <Text
                style={[
                  styles.value,
                  {
                    color:
                      valueTone === "danger"
                        ? colors.danger
                        : colors.textSecondary,
                    textAlign: stacked ? "left" : "right",
                  },
                ]}
              >
                {value}
              </Text>
            ) : null}
            {accessory}
          </View>
        ) : null}
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
        onLayout={measureRow}
        style={[styles.row, fill]}
      >
        {content}
      </View>
    );
  return (
    <Pressable
      accessibilityRole={accessibilityRole ?? "button"}
      accessibilityLabel={
        accessibilityLabel ?? [label, badge, value].filter(Boolean).join(", ")
      }
      accessibilityHint={
        accessibilityHint ??
        (chevron ? t("Opens {{label}}", { label }) : undefined)
      }
      accessibilityState={{ disabled, ...accessibilityState }}
      disabled={disabled}
      onPress={onPress}
      onLayout={measureRow}
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

export type MenuOption<Value extends string> = {
  value: Value;
  label: string;
  detail?: string;
};

// Room kept clear of the status bar and home indicator.
const MENU_MARGIN = 56;
const MENU_MIN_WIDTH = 240;

/**
 * The in-app pop-up menu (an iOS pop-up button's menu): a small floating list
 * anchored under its trigger, right-aligned with it, flipped above it near the
 * bottom of the screen, scrolling when the options do not fit. The current
 * option carries a checkmark; tapping outside closes it.
 */
export function usePopupMenu<Value extends string>({
  options,
  value,
  onChange,
  disabled = false,
}: {
  options: readonly MenuOption<Value>[];
  value: Value;
  onChange: (value: Value) => void;
  disabled?: boolean;
}) {
  const { t } = useAppLanguage();
  const { colors } = useTheme();
  const { width: windowWidth, height, fontScale = 1 } = useWindowDimensions();
  const menuWidth = Math.min(340, windowWidth - 32);
  const anchor = useRef<View>(null);
  const placementGeneration = useRef(0);
  const mounted = useRef(false);
  const currentViewport = useRef({ disabled, width: windowWidth, height });
  currentViewport.current = { disabled, width: windowWidth, height };
  const [menu, setMenu] = useState<{
    generation: number;
    anchorY: number;
    anchorHeight: number;
    left?: number;
    right?: number;
    contentHeight?: number;
  } | null>(null);
  // A menu never outlives its authority or a window resize that moved its anchor.
  useLayoutEffect(() => {
    mounted.current = true;
    placementGeneration.current++;
    setMenu(null);
    return () => {
      mounted.current = false;
      placementGeneration.current++;
    };
  }, [disabled, windowWidth, height]);
  const open = () => {
    if (disabled) return;
    const generation = ++placementGeneration.current;
    // Align with the trigger's nearer edge so the menu stays on screen.
    const place = (
      y: number,
      rowHeight: number,
      side: { left: number } | { right: number },
    ) => {
      if (
        !mounted.current ||
        generation !== placementGeneration.current ||
        currentViewport.current.disabled ||
        currentViewport.current.width !== windowWidth ||
        currentViewport.current.height !== height
      )
        return;
      setMenu({ generation, anchorY: y, anchorHeight: rowHeight, ...side });
    };
    if (!anchor.current?.measureInWindow) {
      place(height / 3, 0, { right: 16 });
      return;
    }
    anchor.current.measureInWindow((x, y, triggerWidth, triggerHeight) =>
      place(
        y,
        triggerHeight,
        x + triggerWidth / 2 < windowWidth / 2
          ? { left: Math.max(8, Math.min(x, windowWidth - menuWidth - 8)) }
          : { right: Math.max(8, windowWidth - (x + triggerWidth) + 8) },
      ),
    );
  };
  const close = () => {
    placementGeneration.current++;
    setMenu(null);
  };
  const below = menu ? menu.anchorY + menu.anchorHeight + 6 : 0;
  const roomBelow = Math.max(0, height - MENU_MARGIN - below);
  const roomAbove = Math.max(0, (menu?.anchorY ?? 0) - 6 - MENU_MARGIN);
  // Native text can wrap beyond the initial estimate. Give the surface an
  // explicit measured height; a flex ScrollView cannot reliably size its
  // absolute parent after text wraps to the final menu width.
  const contentHeight =
    menu?.contentHeight ??
    options.reduce(
      (sum, option) => sum + (option.detail ? 68 : 48) * Math.max(1, fontScale),
      0,
    ) + 8;
  const intrinsicHeight = contentHeight + 2 * StyleSheet.hairlineWidth;
  const flip = intrinsicHeight > roomBelow && roomAbove > roomBelow;
  const maxHeight = flip ? roomAbove : roomBelow;
  const position = {
    ...(flip ? { bottom: height - (menu?.anchorY ?? 0) + 6 } : { top: below }),
    height: Math.min(intrinsicHeight, maxHeight),
    maxHeight,
  };
  const popup = (
    <Modal
      transparent
      visible={!!menu}
      animationType="fade"
      onRequestClose={close}
    >
      <Pressable
        accessibilityRole="button"
        accessibilityLabel={t("Close menu")}
        style={styles.menuBackdrop}
        onPress={close}
      />
      {menu ? (
        <View
          accessibilityRole="menu"
          accessibilityViewIsModal
          style={[
            styles.menu,
            {
              ...position,
              left: menu.left,
              right: menu.right,
              width: menuWidth,
              minWidth: Math.min(MENU_MIN_WIDTH, menuWidth),
              maxWidth: menuWidth,
              backgroundColor: colors.menuFill,
              borderColor: colors.borderSoft,
              shadowColor: "#000",
            },
          ]}
        >
          <ScrollView
            bounces={false}
            showsVerticalScrollIndicator
            onContentSizeChange={(_width, contentHeight) =>
              setMenu((current) =>
                current &&
                current.generation === menu.generation &&
                current.generation === placementGeneration.current &&
                current.contentHeight !== contentHeight
                  ? { ...current, contentHeight }
                  : current,
              )
            }
            style={styles.menuClip}
          >
            {options.map((option, index) => {
              const selected = option.value === value;
              return (
                <Pressable
                  key={option.value}
                  accessibilityRole="menuitem"
                  accessibilityState={{ selected, checked: selected }}
                  accessibilityLabel={
                    option.detail
                      ? `${option.label}. ${option.detail}`
                      : option.label
                  }
                  onPress={() => {
                    close();
                    if (!disabled && !selected) onChange(option.value);
                  }}
                  style={({ pressed }) => [
                    styles.menuItem,
                    index > 0 && {
                      borderTopWidth: StyleSheet.hairlineWidth,
                      borderTopColor: colors.borderSoft,
                    },
                    pressed && { backgroundColor: colors.navActive },
                  ]}
                >
                  <View style={styles.menuCheck}>
                    {selected ? (
                      <Check
                        size={17}
                        color={colors.textPrimary}
                        strokeWidth={2.5}
                      />
                    ) : null}
                  </View>
                  <View style={styles.menuText}>
                    <Text
                      style={[
                        styles.label,
                        { color: colors.textPrimary },
                        selected && styles.menuSelected,
                      ]}
                    >
                      {option.label}
                    </Text>
                    {option.detail ? (
                      <Text
                        style={[styles.detail, { color: colors.textMuted }]}
                      >
                        {option.detail}
                      </Text>
                    ) : null}
                  </View>
                </Pressable>
              );
            })}
          </ScrollView>
        </View>
      ) : null}
    </Modal>
  );
  return { anchor, open, popup, isOpen: !!menu };
}

/**
 * A choice among a few options as one row: the label, then the current value
 * with a pop-up chevron beside it at the trailing edge. Tapping opens the
 * shared in-app menu. Use this instead of spreading options across rows.
 */
export function SettingsMenuRow<Value extends string>({
  icon,
  label,
  detail,
  value,
  options,
  onChange,
  disabled = false,
}: {
  icon?: LucideIcon;
  label: string;
  detail?: string;
  value: Value;
  options: readonly MenuOption<Value>[];
  onChange: (value: Value) => void;
  disabled?: boolean;
}) {
  const { t } = useAppLanguage();
  const { colors } = useTheme();
  const menu = usePopupMenu({ options, value, onChange, disabled });
  const current = options.find((option) => option.value === value);
  return (
    <View ref={menu.anchor} collapsable={false}>
      <SettingsRow
        icon={icon}
        label={label}
        detail={detail}
        value={current?.label ?? value}
        disabled={disabled}
        accessibilityHint={t("Opens a menu to choose {{label}}", {
          label: label.toLowerCase(),
        })}
        accessory={
          <ChevronsUpDown
            size={15}
            color={disabled ? colors.textMuted : colors.textSecondary}
          />
        }
        onPress={menu.open}
      />
      {menu.popup}
    </View>
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
  const { t } = useAppLanguage();
  label = t(label);
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
  headerAction: {
    minHeight: 44,
    justifyContent: "center",
    paddingHorizontal: 4,
  },
  menuBackdrop: { ...StyleSheet.absoluteFillObject },
  menu: {
    position: "absolute",
    minWidth: MENU_MIN_WIDTH,
    maxWidth: 300,
    borderRadius: 14,
    borderWidth: StyleSheet.hairlineWidth,
    shadowOpacity: 0.18,
    shadowRadius: 24,
    shadowOffset: { width: 0, height: 8 },
    elevation: 12,
  },
  menuItem: {
    flexDirection: "row",
    alignItems: "center",
    gap: 10,
    minHeight: 48,
    paddingHorizontal: 14,
    paddingVertical: 11,
  },
  // Clipping lives inside so the outer surface keeps its iOS shadow.
  menuClip: { borderRadius: 14, overflow: "hidden" },
  menuCheck: { width: 18, alignItems: "center" },
  menuText: { flex: 1, minWidth: 0, gap: 2 },
  menuSelected: { fontFamily: fonts.bodySemi, fontWeight: "600" },
  headerActionLabel: {
    ...typeScale.label,
    fontFamily: fonts.bodySemi,
    fontWeight: "600",
  },
  pageContent: {
    width: "100%",
    maxWidth: 720,
    alignSelf: "center",
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
  labelLine: {
    flexDirection: "row",
    flexWrap: "wrap",
    alignItems: "center",
    columnGap: spacing.sm,
    rowGap: 2,
  },
  label: { ...typeScale.label },
  labelText: { flexShrink: 1 },
  badge: {
    borderRadius: 6,
    paddingHorizontal: 6,
    paddingVertical: 2,
  },
  badgeText: { ...typeScale.small, fontSize: 10, lineHeight: 13 },
  centeredLabel: { textAlign: "center" },
  detail: { ...typeScale.small },
  trailing: {
    flexDirection: "row",
    alignItems: "center",
    gap: spacing.xs + 2,
    flexShrink: 0,
    maxWidth: "55%",
  },
  trailingStacked: { maxWidth: "100%" },
  value: { ...typeScale.description, flexShrink: 1 },
});
