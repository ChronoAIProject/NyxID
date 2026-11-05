import {
  useEffect,
  useState,
  type ComponentProps,
  type ReactNode,
} from "react";
import {
  Keyboard,
  Pressable,
  StyleSheet,
  Text,
  TextInput,
  useWindowDimensions,
  View,
} from "react-native";
import { useSafeAreaInsets } from "react-native-safe-area-context";
import { ArrowUp, Mic, PhoneOff } from "lucide-react-native";
import { useTheme } from "../../../theme/ThemeContext";
import { fonts } from "../../../theme/designTokens";
import { ChatComposerFrame } from "../ChatComposer";
import { VoiceComposerAura } from "./VoiceComposerAura";

/** Reserve transcript space above either voice composer, including with the keyboard open. */
export function useVoiceComposerHeight() {
  const { height } = useWindowDimensions();
  const insets = useSafeAreaInsets();
  const [keyboardTop, setKeyboardTop] = useState<number>();
  useEffect(() => {
    const show = Keyboard.addListener("keyboardDidShow", (event) =>
      setKeyboardTop(event.endCoordinates.screenY),
    );
    const hide = Keyboard.addListener("keyboardDidHide", () =>
      setKeyboardTop(undefined),
    );
    return () => {
      show.remove();
      hide.remove();
    };
  }, []);
  return Math.max(
    88,
    Math.min(height, keyboardTop ?? height) -
      insets.top -
      insets.bottom -
      60 -
      96,
  );
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
  return (
    <View style={styles.header}>
      <View style={styles.headerText}>
        <Text
          numberOfLines={1}
          style={[styles.title, { color: colors.textPrimary }]}
        >
          {name}
        </Text>
        <Text
          numberOfLines={1}
          style={[
            styles.status,
            { color: error ? colors.danger : colors.textSecondary },
          ]}
        >
          <Text accessibilityLiveRegion="polite">{status}</Text>
          <Text
            accessibilityLabel={`Call time ${duration}`}
            style={[styles.elapsed, { color: colors.textSecondary }]}
          >{`  ${duration}`}</Text>
        </Text>
      </View>
      {children}
    </View>
  );
}

export function VoiceComposerTextRow({
  minimized,
  status,
  duration,
  draft = "",
  onExpand,
  onChangeText,
  onSendText,
  canSendText,
  onClose,
  inputLabel = "Message during voice",
}: {
  minimized: boolean;
  status: string;
  duration: string;
  draft?: string;
  onExpand?: () => void;
  onChangeText?: (text: string) => void;
  onSendText?: () => void;
  canSendText?: boolean;
  onClose: () => void;
  inputLabel?: string;
}) {
  const { colors } = useTheme();
  return (
    <View style={styles.compactRow}>
      <Pressable
        accessibilityRole="button"
        accessibilityLabel="Expand voice composer"
        onPress={onExpand}
        style={styles.compactExpand}
      >
        <Mic size={20} color={colors.textPrimary} />
        {minimized && !onChangeText ? (
          <Text
            numberOfLines={1}
            style={[styles.status, { color: colors.textPrimary }]}
          >
            {status}
          </Text>
        ) : null}
      </Pressable>
      {onChangeText ? (
        <TextInput
          accessibilityLabel={inputLabel}
          value={draft}
          onFocus={onExpand}
          onChangeText={(text) => {
            onExpand?.();
            onChangeText(text);
          }}
          placeholder={minimized ? status : "Message"}
          placeholderTextColor={colors.textSecondary}
          style={[styles.voiceInput, { color: colors.textPrimary }]}
          returnKeyType="send"
          onSubmitEditing={() => {
            if (canSendText) onSendText?.();
          }}
        />
      ) : null}
      {minimized ? (
        <Text
          accessibilityLabel={`Call time ${duration}`}
          style={[styles.elapsed, { color: colors.textSecondary }]}
        >
          {duration}
        </Text>
      ) : null}
      {draft.trim() && onSendText ? (
        <VoiceIconButton
          label="Send message during voice"
          disabled={!canSendText}
          onPress={onSendText}
        >
          <ArrowUp size={20} color={colors.textPrimary} />
        </VoiceIconButton>
      ) : null}
      {minimized ? (
        <VoiceIconButton label="End voice and close" onPress={onClose}>
          <PhoneOff size={20} color={colors.danger} />
        </VoiceIconButton>
      ) : null}
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
  compactFrame: { paddingHorizontal: 8, paddingVertical: 4 },
  header: {
    flexDirection: "row",
    alignItems: "center",
    gap: 10,
    minHeight: 36,
    paddingLeft: 4,
  },
  headerText: { flex: 1, minWidth: 0 },
  title: { fontFamily: fonts.bodySemi, fontSize: 15, lineHeight: 20 },
  status: { fontFamily: fonts.body, fontSize: 13, lineHeight: 18 },
  elapsed: { fontFamily: fonts.mono, fontSize: 12, lineHeight: 18 },
  compactRow: { flexDirection: "row", alignItems: "center", minHeight: 44 },
  compactExpand: {
    flexDirection: "row",
    gap: 10,
    alignItems: "center",
    minHeight: 44,
    paddingHorizontal: 8,
  },
  voiceInput: {
    flex: 1,
    minWidth: 0,
    minHeight: 44,
    fontFamily: fonts.body,
    fontSize: 14,
  },
  controls: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
    paddingHorizontal: 8,
  },
  icon: {
    width: 36,
    height: 36,
    borderRadius: 18,
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
