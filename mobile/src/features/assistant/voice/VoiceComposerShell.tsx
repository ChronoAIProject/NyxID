import { type ComponentProps, type ReactNode } from "react";
import {
  Pressable,
  StyleSheet,
  Text,
  useWindowDimensions,
  View,
} from "react-native";
import { useSafeAreaInsets } from "react-native-safe-area-context";
import { PhoneOff } from "lucide-react-native";
import { useTheme } from "../../../theme/ThemeContext";
import { fonts } from "../../../theme/designTokens";
import { ChatComposerFrame } from "../ChatComposer";
import {
  DictationWaveform,
  type LevelSubscription,
} from "../DictationWaveform";
import { composerVoiceMinHeight } from "../composerGeometry";
import { VoiceComposerAura } from "./VoiceComposerAura";

/** Leave transcript space while bounding voice to the current phone/tablet window. */
export function useVoiceComposerHeight() {
  const { height } = useWindowDimensions();
  const insets = useSafeAreaInsets();
  return Math.max(88, height - insets.top - insets.bottom - 60 - 96);
}

/** The same voice surface surrounds native crew speech and private realtime calls. */
export function VoiceComposerShell({
  minimized,
  children,
  ...aura
}: ComponentProps<typeof VoiceComposerAura> & { minimized: boolean }) {
  return (
    <VoiceComposerAura {...aura}>
      <ChatComposerFrame
        style={minimized ? styles.compactFrame : styles.frame}
        bounce={false}
      >
        {children}
      </ChatComposerFrame>
    </VoiceComposerAura>
  );
}

export function VoiceComposerHeader({
  name,
  status,
  duration,
  error,
  children,
}: {
  name: string;
  status: string;
  duration: string;
  error?: boolean;
  children?: ReactNode;
}) {
  const { colors } = useTheme();
  const { fontScale } = useWindowDimensions();
  const separateTitle = fontScale > 1.8 && !!children;
  return (
    <View style={styles.header}>
      <View style={styles.headerTop}>
        {!separateTitle ? (
          <Text
            numberOfLines={1}
            style={[
              styles.title,
              styles.headerText,
              { color: colors.textPrimary },
            ]}
          >
            {name}
          </Text>
        ) : null}
        {children}
      </View>
      {separateTitle ? (
        <Text
          numberOfLines={1}
          style={[styles.title, { color: colors.textPrimary }]}
        >
          {name}
        </Text>
      ) : null}
      <View style={styles.statusRow}>
        <Text
          numberOfLines={1}
          accessibilityLiveRegion="polite"
          style={[
            styles.status,
            styles.headerText,
            { color: error ? colors.danger : colors.textSecondary },
          ]}
        >
          {status}
        </Text>
        <Text
          accessibilityLabel={`Call time ${duration}`}
          style={[styles.elapsed, { color: colors.textSecondary }]}
        >
          {duration}
        </Text>
      </View>
    </View>
  );
}

export function VoiceComposerCompactRow({
  status,
  duration,
  liveText,
  instruction,
  controls,
  crew = false,
  subscribeLevel,
  onExpand,
  onClose,
}: {
  status: string;
  duration: string;
  liveText?: string;
  instruction?: string;
  controls?: ReactNode;
  crew?: boolean;
  subscribeLevel: LevelSubscription;
  onExpand?: () => void;
  onClose: () => void;
}) {
  const { colors } = useTheme();
  const { fontScale = 1 } = useWindowDimensions();
  if (controls) {
    return (
      <View
        testID="voice-compact-row"
        style={[
          styles.compactHoldRow,
          { minHeight: composerVoiceMinHeight(fontScale, crew) },
        ]}
      >
        <Pressable
          accessibilityRole="button"
          accessibilityLabel="Expand voice composer"
          accessibilityHint={`${status}. Call time ${duration}. Tap to expand for listening mode options${liveText ? `. ${liveText}` : ""}`}
          onPress={onExpand}
          style={styles.compactHoldExpand}
        />
        <View pointerEvents="none" style={styles.compactHoldTop}>
          <Text
            numberOfLines={1}
            style={[styles.compactHoldSpeech, { color: colors.textPrimary }]}
          >
            {liveText || status}
          </Text>
          <Text
            accessibilityLabel={`Call time ${duration}`}
            style={[styles.elapsed, { color: colors.textSecondary }]}
          >
            {duration}
          </Text>
        </View>
        <View pointerEvents="box-none" style={styles.compactHoldBottom}>
          <View pointerEvents="none" style={styles.compactHoldMeter}>
            <Text
              numberOfLines={1}
              style={[
                styles.compactHoldInstruction,
                { color: colors.textSecondary },
              ]}
            >
              {instruction}
            </Text>
            <DictationWaveform subscribe={subscribeLevel} />
          </View>
          {controls}
          <VoiceIconButton label="End voice and close" onPress={onClose}>
            <PhoneOff size={20} color={colors.danger} />
          </VoiceIconButton>
        </View>
      </View>
    );
  }
  return (
    <View
      testID="voice-compact-row"
      style={[
        styles.compactRow,
        { minHeight: composerVoiceMinHeight(fontScale, crew) },
      ]}
    >
      <Pressable
        accessibilityRole="button"
        accessibilityLabel="Expand voice composer"
        accessibilityHint={`${status}. Call time ${duration}. Tap to expand for listening mode options${liveText ? `. ${liveText}` : ""}`}
        onPress={onExpand}
        style={styles.compactExpand}
      >
        <View style={styles.compactMeter}>
          {liveText ? (
            <Text
              numberOfLines={1}
              style={[styles.liveSpeech, { color: colors.textPrimary }]}
            >
              {liveText}
            </Text>
          ) : null}
          {instruction ? (
            <Text style={[styles.status, { color: colors.textSecondary }]}>
              {instruction}
            </Text>
          ) : null}
          <View style={styles.waveRow}>
            <DictationWaveform subscribe={subscribeLevel} />
          </View>
        </View>
        <Text
          accessibilityLabel={`Call time ${duration}`}
          style={[styles.elapsed, { color: colors.textSecondary }]}
        >
          {duration}
        </Text>
      </Pressable>
      {controls}
      <VoiceIconButton label="End voice and close" onPress={onClose}>
        <PhoneOff size={20} color={colors.danger} />
      </VoiceIconButton>
    </View>
  );
}

export function VoiceComposerControls({ children }: { children: ReactNode }) {
  return <View style={styles.controls}>{children}</View>;
}

export function VoiceIconButton({
  label,
  selected,
  disabled = false,
  large = false,
  onPress,
  children,
}: {
  label: string;
  selected?: boolean;
  disabled?: boolean;
  large?: boolean;
  onPress: () => void;
  children: ReactNode;
}) {
  const { colors } = useTheme();
  return (
    <Pressable
      accessibilityRole="button"
      accessibilityLabel={label}
      accessibilityState={{ disabled, selected: !!selected }}
      disabled={disabled}
      hitSlop={large ? 4 : 6}
      onPress={onPress}
      style={({ pressed }) => [
        large ? styles.iconLarge : styles.icon,
        {
          backgroundColor: selected
            ? colors.segmentSelected
            : colors.segmentTrack,
          opacity: disabled ? 0.4 : pressed ? 0.7 : 1,
        },
      ]}
    >
      {children}
    </Pressable>
  );
}

export function VoiceMainButton({
  label,
  fill,
  disabled,
  onPress,
  children,
}: {
  label: string;
  fill: string;
  disabled: boolean;
  onPress: () => void;
  children: ReactNode;
}) {
  return (
    <Pressable
      accessibilityRole="button"
      accessibilityLabel={label}
      accessibilityState={{ disabled }}
      disabled={disabled}
      onPress={onPress}
      style={({ pressed }) => [
        styles.main,
        { backgroundColor: fill, opacity: disabled ? 0.4 : pressed ? 0.8 : 1 },
      ]}
    >
      {children}
    </Pressable>
  );
}

const styles = StyleSheet.create({
  frame: { gap: 10, paddingHorizontal: 12, paddingTop: 12, paddingBottom: 10 },
  compactFrame: { paddingHorizontal: 4, paddingVertical: 4 },
  header: { gap: 2, minHeight: 36, paddingLeft: 4 },
  headerTop: {
    flexDirection: "row",
    alignItems: "center",
    gap: 8,
    minHeight: 50,
  },
  statusRow: { flexDirection: "row", alignItems: "center", gap: 8 },
  headerText: { flex: 1, minWidth: 0 },
  title: { fontFamily: fonts.bodySemi, fontSize: 15, lineHeight: 20 },
  status: { fontFamily: fonts.body, fontSize: 13, lineHeight: 18 },
  elapsed: { fontFamily: fonts.mono, fontSize: 12, lineHeight: 18 },
  compactHoldRow: {
    paddingHorizontal: 12,
    paddingVertical: 4,
    gap: 4,
    justifyContent: "center",
  },
  compactHoldExpand: {
    ...StyleSheet.absoluteFillObject,
    minWidth: 44,
    minHeight: 44,
  },
  compactHoldTop: { flexDirection: "row", alignItems: "center", gap: 12 },
  compactHoldSpeech: {
    flex: 1,
    minWidth: 0,
    fontFamily: fonts.bodyRegular,
    fontSize: 16,
    lineHeight: 20,
  },
  compactHoldBottom: { flexDirection: "row", alignItems: "center", gap: 8 },
  compactHoldMeter: {
    flex: 1,
    minWidth: 0,
    flexDirection: "row",
    alignItems: "center",
    gap: 8,
    minHeight: 44,
  },
  compactHoldInstruction: {
    flexShrink: 1,
    fontFamily: fonts.bodySemi,
    fontSize: 12,
    lineHeight: 16,
  },
  compactRow: { flexDirection: "row", alignItems: "center", minHeight: 44 },
  compactExpand: {
    flex: 1,
    minWidth: 44,
    minHeight: 44,
    flexDirection: "row",
    gap: 12,
    alignItems: "center",
    paddingHorizontal: 12,
    paddingVertical: 8,
  },
  compactMeter: { flex: 1, minWidth: 0, gap: 6 },
  waveRow: { height: 22, flexDirection: "row", alignItems: "center" },
  liveSpeech: { fontFamily: fonts.bodyRegular, fontSize: 16, lineHeight: 22 },
  controls: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
    paddingHorizontal: 8,
  },
  icon: {
    width: 44,
    height: 44,
    borderRadius: 22,
    alignItems: "center",
    justifyContent: "center",
  },
  iconLarge: {
    width: 48,
    height: 48,
    borderRadius: 24,
    alignItems: "center",
    justifyContent: "center",
  },
  main: {
    width: 64,
    height: 64,
    borderRadius: 32,
    alignItems: "center",
    justifyContent: "center",
  },
});
