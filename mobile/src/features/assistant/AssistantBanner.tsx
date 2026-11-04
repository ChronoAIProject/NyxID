import { useEffect, useRef, useState, type ReactNode } from "react";
import { Animated, AppState, Pressable, View } from "react-native";
import {
  ArrowDown,
  Info,
  TriangleAlert,
  Wifi,
  WifiOff,
  X,
} from "lucide-react-native";
import { useIsFocused } from "@react-navigation/native";
import { useTheme } from "../../theme/ThemeContext";
import { useReducedMotion } from "../../lib/accessibility/useReducedMotion";
import { Label } from "../../components/workspace/WorkspaceUI";
import { useNetworkStatus } from "../../hooks/useNetworkStatus";
import { assistantConnection } from "../../lib/api/connectionState";

export function AssistantBanner({
  children,
  tone = "neutral",
  icon,
  onDismiss,
}: {
  children: ReactNode;
  tone?: "neutral" | "error";
  icon?: ReactNode;
  onDismiss?: () => void;
}) {
  const { colors } = useTheme();
  return (
    <View
      accessibilityRole={tone === "error" ? "alert" : undefined}
      accessibilityLiveRegion="polite"
      style={{
        borderWidth: 1,
        borderColor:
          tone === "error" ? colors.dangerTone.border : colors.border,
        backgroundColor:
          tone === "error" ? colors.dangerTone.bg : colors.cardSoft,
        borderRadius: 8,
        flexDirection: "row",
        alignItems: "center",
        minHeight: 32,
        paddingHorizontal: 12,
        paddingVertical: 6,
        gap: 8,
      }}
    >
      {icon ??
        (tone === "error" ? (
          <TriangleAlert size={14} color={colors.dangerTone.text} />
        ) : (
          <Info size={14} color={colors.textMuted} />
        ))}
      <View style={{ flex: 1, minWidth: 0 }}>
        {typeof children === "string" ? (
          <Label
            style={{
              fontSize: 11,
              lineHeight: 17,
              color:
                tone === "error"
                  ? colors.dangerTone.text
                  : colors.textSecondary,
            }}
          >
            {children}
          </Label>
        ) : (
          children
        )}
      </View>
      {onDismiss ? (
        <Pressable
          accessibilityRole="button"
          accessibilityLabel="Dismiss"
          style={{
            width: 44,
            minHeight: 44,
            alignItems: "center",
            justifyContent: "center",
          }}
          onPress={onDismiss}
        >
          <X size={14} color={colors.dangerTone.text} />
        </Pressable>
      ) : null}
    </View>
  );
}
export function AssistantErrorNotice({ message }: { message?: string }) {
  const [dismissed, setDismissed] = useState<string>();
  if (!message || message === dismissed) return null;
  return (
    <AssistantBanner tone="error" onDismiss={() => setDismissed(message)}>
      {message}
    </AssistantBanner>
  );
}
const AnimatedPressable = Animated.createAnimatedComponent(Pressable);

export function JumpToLatest({
  visible,
  onPress,
}: {
  visible: boolean;
  onPress: () => void;
}) {
  const { colors } = useTheme();
  const reducedMotion = useReducedMotion();
  const [offset] = useState(() => new Animated.Value(0));
  if (!visible) return null;
  // The arrow dips toward the latest message, then springs back as the
  // transcript scrolls there. Reduce Motion scrolls at once.
  const jump = () => {
    if (reducedMotion) return onPress();
    offset.stopAnimation();
    Animated.timing(offset, {
      toValue: 6,
      duration: 90,
      useNativeDriver: true,
    }).start(() => {
      onPress();
      Animated.spring(offset, {
        toValue: 0,
        speed: 24,
        bounciness: 14,
        useNativeDriver: true,
      }).start();
    });
  };
  return (
    <View
      pointerEvents="box-none"
      style={{
        position: "absolute",
        bottom: 0,
        left: 0,
        right: 0,
        height: 56,
        zIndex: 20,
        alignItems: "center",
      }}
    >
      <AnimatedPressable
        accessibilityRole="button"
        accessibilityLabel="Jump to latest message"
        hitSlop={6}
        onPress={jump}
        style={{
          transform: [{ translateY: offset }],
          position: "absolute",
          bottom: 12,
          width: 32,
          height: 32,
          alignItems: "center",
          justifyContent: "center",
          borderRadius: 16,
          borderWidth: 0.5,
          borderColor: colors.border,
          backgroundColor: `${colors.bg}f2`,
          shadowColor: "#000000",
          shadowOpacity: 0.12,
          shadowOffset: { width: 0, height: 1 },
          shadowRadius: 2,
          elevation: 2,
        }}
      >
        <ArrowDown size={16} color={colors.textMuted} />
      </AnimatedPressable>
    </View>
  );
}
export function AssistantConnectionBanner({
  onRetry,
  inset = false,
}: {
  onRetry: () => void;
  inset?: boolean;
}) {
  const { colors } = useTheme();
  const focused = useIsFocused();
  const { isConnected } = useNetworkStatus();
  const [unreachable, setUnreachable] = useState(assistantConnection.snapshot);
  const [reconnected, setReconnected] = useState(false);
  const wasDown = useRef(!isConnected || unreachable);
  const latest = useRef(onRetry);
  latest.current = onRetry;
  useEffect(
    () =>
      assistantConnection.subscribe(() =>
        setUnreachable(assistantConnection.snapshot()),
      ),
    [],
  );
  const down = !isConnected || unreachable;
  useEffect(() => {
    let timer: ReturnType<typeof setTimeout> | undefined;
    if (down) setReconnected(false);
    else if (wasDown.current) {
      setReconnected(true);
      latest.current();
      timer = setTimeout(() => setReconnected(false), 3000);
    }
    wasDown.current = down;
    return () => clearTimeout(timer);
  }, [down]);
  useEffect(() => {
    if (!focused || !isConnected || !unreachable) return;
    const timer = setInterval(() => {
      if (AppState.currentState === "active") latest.current();
    }, 5000);
    return () => clearInterval(timer);
  }, [focused, isConnected, unreachable]);
  if (!focused || (!down && !reconnected)) return null;
  return (
    <View style={inset ? { marginHorizontal: 12 } : undefined}>
      <AssistantBanner
        icon={
          down ? (
            <WifiOff size={14} color={colors.warningTone.text} />
          ) : (
            <Wifi size={14} color={colors.successTone.text} />
          )
        }
      >
        {!isConnected
          ? "You're offline. NyxBot can't send, run or refresh until you reconnect. You're seeing what was already loaded."
          : unreachable
            ? "Can't reach NyxID. NyxBot can't send, run or refresh until the connection is back. You're seeing what was already loaded."
            : "Back online."}
      </AssistantBanner>
    </View>
  );
}
