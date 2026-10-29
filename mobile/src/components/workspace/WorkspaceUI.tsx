import type { PropsWithChildren } from "react";
import {
  ActivityIndicator,
  KeyboardAvoidingView,
  Platform,
  Pressable,
  RefreshControl,
  ScrollView,
  StyleSheet,
  Text,
  TextInput,
  View,
  type StyleProp,
  type TextInputProps,
  type TextProps,
  type ViewStyle,
} from "react-native";
import { LinearGradient } from "expo-linear-gradient";
import {
  SafeAreaView,
  useSafeAreaInsets,
} from "react-native-safe-area-context";
import { useTheme } from "../../theme/ThemeContext";
import { fonts, radius, TOUCH_TARGET } from "../../theme/designTokens";
import { AndroidKeyboardAvoidingView } from "./AndroidKeyboardAvoidingView";

export function WorkspacePage({
  title,
  titleNumberOfLines,
  subtitle,
  children,
  scroll = true,
  showHeading = true,
  refreshing = false,
  onRefresh,
  contentStyle,
}: PropsWithChildren<{
  title: string;
  titleNumberOfLines?: number;
  subtitle?: string;
  scroll?: boolean;
  showHeading?: boolean;
  refreshing?: boolean;
  onRefresh?: () => void;
  contentStyle?: StyleProp<ViewStyle>;
}>) {
  const { colors } = useTheme();
  const insets = useSafeAreaInsets();
  const content = (
    <>
      {showHeading && (
        <View style={styles.heading}>
          <Text
            accessibilityRole="header"
            accessibilityLabel={title}
            accessibilityActions={
              onRefresh ? [{ name: "refresh", label: "Refresh" }] : undefined
            }
            onAccessibilityAction={
              onRefresh
                ? ({ nativeEvent }) => {
                    if (nativeEvent.actionName === "refresh" && !refreshing) {
                      onRefresh();
                    }
                  }
                : undefined
            }
            numberOfLines={titleNumberOfLines}
            ellipsizeMode="tail"
            style={[styles.title, { color: colors.textPrimary }]}
          >
            {title}
          </Text>
          {subtitle ? (
            <Text style={[styles.subtitle, { color: colors.textSecondary }]}>
              {subtitle}
            </Text>
          ) : null}
        </View>
      )}
      {children}
    </>
  );
  const body = scroll ? (
    <ScrollView
      keyboardShouldPersistTaps="handled"
      keyboardDismissMode="on-drag"
      contentContainerStyle={[styles.content, contentStyle]}
      alwaysBounceVertical={onRefresh ? true : undefined}
      refreshControl={
        onRefresh ? (
          <RefreshControl
            accessibilityLabel="Refresh"
            refreshing={refreshing}
            onRefresh={onRefresh}
            tintColor={colors.primary}
            colors={[colors.primary]}
            progressBackgroundColor={colors.card}
          />
        ) : undefined
      }
    >
      {content}
    </ScrollView>
  ) : (
    <View style={[styles.content, styles.fill, contentStyle]}>{content}</View>
  );
  return (
    <SafeAreaView
      edges={["bottom", "left", "right"]}
      style={[styles.fill, { backgroundColor: colors.bg }]}
    >
      {Platform.OS === "android" ? (
        <AndroidKeyboardAvoidingView>{body}</AndroidKeyboardAvoidingView>
      ) : (
        <KeyboardAvoidingView
          style={styles.fill}
          behavior={Platform.OS === "ios" ? "padding" : undefined}
          keyboardVerticalOffset={insets.top + 56}
        >
          {body}
        </KeyboardAvoidingView>
      )}
    </SafeAreaView>
  );
}

export function Card({
  children,
  style,
}: PropsWithChildren<{ style?: StyleProp<ViewStyle> }>) {
  const { colors } = useTheme();
  return (
    <View
      style={[
        styles.card,
        { backgroundColor: colors.card, borderColor: colors.border },
        style,
      ]}
    >
      {children}
    </View>
  );
}

export function Label({
  children,
  muted,
  mono,
  style,
  ...props
}: TextProps & { muted?: boolean; mono?: boolean }) {
  const { colors } = useTheme();
  return (
    <Text
      {...props}
      style={[
        styles.label,
        {
          color: muted ? colors.textSecondary : colors.textPrimary,
          fontFamily: mono ? fonts.mono : fonts.body,
        },
        style,
      ]}
    >
      {children}
    </Text>
  );
}

export function Field({
  label,
  style,
  ...props
}: TextInputProps & { label: string }) {
  const { colors } = useTheme();
  return (
    <View style={styles.field}>
      <Label>{label}</Label>
      <TextInput
        accessibilityLabel={label}
        placeholderTextColor={colors.textMuted}
        selectionColor={colors.primary}
        autoCapitalize="none"
        {...props}
        style={[
          styles.input,
          {
            color: colors.textPrimary,
            borderColor: colors.controlBorder,
            backgroundColor: colors.cardSoft,
          },
          props.multiline && styles.multiline,
          style,
        ]}
      />
    </View>
  );
}

export function ActionButton({
  label,
  accessibilityLabel,
  onPress,
  disabled,
  loading,
  compact = false,
  variant = "primary",
}: {
  label: string;
  accessibilityLabel?: string;
  onPress: () => void;
  disabled?: boolean;
  loading?: boolean;
  compact?: boolean;
  variant?: "primary" | "secondary" | "danger";
}) {
  const { colors } = useTheme();
  const blocked = disabled || loading;
  const color =
    blocked && !loading
      ? colors.textTertiary
      : variant === "primary"
      ? colors.onPrimary
      : variant === "danger"
        ? colors.danger
        : colors.textPrimary;
  const content = (
    <View style={styles.buttonContent}>
      {loading ? <ActivityIndicator size="small" color={color} /> : null}
      <Text
        style={[
          styles.buttonLabel,
          compact && styles.compactButtonLabel,
          { color },
        ]}
      >
        {label}
      </Text>
    </View>
  );
  return (
    <Pressable
      accessibilityRole="button"
      accessibilityLabel={accessibilityLabel ?? label}
      accessibilityState={{ disabled: !!blocked, busy: !!loading }}
      onPress={onPress}
      disabled={blocked}
      style={({ pressed }) => [
        styles.button,
        compact && styles.compactButton,
        { opacity: pressed && !blocked ? 0.8 : 1 },
      ]}
    >
      {variant === "primary" && !(disabled && !loading) ? (
        <LinearGradient
          colors={[colors.gradientStart, colors.gradientEnd]}
          start={{ x: 0, y: 0 }}
          end={{ x: 1, y: 0 }}
          style={[styles.buttonFill, compact && styles.compactButtonFill]}
        >
          {content}
        </LinearGradient>
      ) : (
        <View
          style={[
            styles.buttonFill,
            compact && styles.compactButtonFill,
            {
              backgroundColor:
                disabled && !loading
                  ? colors.cardSoft
                  : variant === "danger"
                    ? colors.dangerSoftBg
                    : compact
                      ? colors.navActive
                      : colors.card,
              borderWidth: compact && variant === "secondary" ? 0 : 1,
              borderColor:
                variant === "danger" ? colors.danger : colors.controlBorder,
            },
          ]}
        >
          {content}
        </View>
      )}
    </Pressable>
  );
}

export function Notice({
  message,
  tone = "info",
}: {
  message: string;
  tone?: "error" | "info" | "success";
}) {
  const { colors } = useTheme();
  const palette =
    tone === "error"
      ? colors.dangerTone
      : tone === "success"
        ? colors.successTone
        : colors.infoTone;
  return (
    <View
      accessibilityLiveRegion="polite"
      style={[
        styles.notice,
        { backgroundColor: palette.bg, borderColor: palette.border },
      ]}
    >
      <Text selectable style={[styles.label, { color: palette.text }]}>
        {message}
      </Text>
    </View>
  );
}

const styles = StyleSheet.create({
  fill: { flex: 1 },
  content: { padding: 20, gap: 16, flexGrow: 1 },
  heading: { gap: 8 },
  title: {
    fontFamily: fonts.displaySemi,
    fontSize: 26,
    lineHeight: 32,
    letterSpacing: -0.6,
  },
  subtitle: { fontFamily: fonts.bodyRegular, fontSize: 13, lineHeight: 20 },
  card: { padding: 16, gap: 12, borderWidth: 1, borderRadius: radius.lg },
  label: { fontFamily: fonts.body, fontSize: 13, lineHeight: 20 },
  field: { gap: 8 },
  input: {
    fontFamily: fonts.bodyRegular,
    fontSize: 14,
    minHeight: TOUCH_TARGET + 4,
    borderWidth: 1,
    borderRadius: radius.md,
    paddingHorizontal: 12,
    paddingVertical: 12,
  },
  multiline: { minHeight: 96, textAlignVertical: "top" },
  button: { borderRadius: radius.md, overflow: "hidden" },
  compactButton: {
    alignSelf: "flex-start",
    maxWidth: "100%",
    minHeight: TOUCH_TARGET,
    justifyContent: "center",
    paddingVertical: 4,
  },
  compactButtonLabel: {
    flexShrink: 1,
    fontFamily: fonts.body,
    fontSize: 13,
    lineHeight: 18,
  },
  compactButtonFill: {
    minHeight: 36,
    paddingHorizontal: 12,
    paddingVertical: 8,
  },
  buttonFill: {
    borderRadius: radius.md,
    minHeight: TOUCH_TARGET + 4,
    justifyContent: "center",
    paddingHorizontal: 16,
    paddingVertical: 10,
  },
  buttonContent: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "center",
    gap: 8,
  },
  buttonLabel: { fontFamily: fonts.bodySemi, fontSize: 13, lineHeight: 20 },
  notice: { padding: 12, borderWidth: 1, borderRadius: radius.md },
});
