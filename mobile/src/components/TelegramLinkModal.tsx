import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  ActivityIndicator,
  Linking,
  Modal,
  Pressable,
  StyleSheet,
  Text,
  View,
} from "react-native";
import * as Clipboard from "expo-clipboard";
import Svg, { Path } from "react-native-svg";
import { useTheme } from "../theme/ThemeContext";
import type { ThemeColors } from "../theme/mobileTheme";
import { TOUCH_TARGET, radius, spacing, typeScale } from "../theme/designTokens";
import { mobileApi } from "../lib/api/mobileApi";
import type { TelegramLinkInfo } from "../lib/api/types";

type TelegramLinkModalProps = {
  visible: boolean;
  onDismiss: () => void;
  onConnected: () => void;
};

const POLL_INTERVAL_MS = 3000;
const POLL_TIMEOUT_MS = 60000;

function TelegramIcon() {
  return (
    <Svg width={18} height={18} viewBox="0 0 24 24" fill="#FFFFFF">
      <Path d="M11.944 0A12 12 0 1 0 24 12.056A12.014 12.014 0 0 0 11.944 0Zm5.654 8.22l-1.69 7.955c-.128.574-.462.713-.937.444l-2.586-1.906l-1.248 1.2a.65.65 0 0 1-.52.254l.186-2.632l4.789-4.326c.208-.186-.045-.29-.324-.103L9.3 13.264l-2.548-.795c-.554-.174-.565-.554.116-.82l9.952-3.836c.462-.166.866.113.716.82Z" />
    </Svg>
  );
}

function CopyIcon({ color }: { color: string }) {
  return (
    <Svg width={14} height={14} viewBox="0 0 24 24" fill="none" stroke={color} strokeWidth={2} strokeLinecap="round" strokeLinejoin="round">
      <Path d="M16 4h2a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2h2" />
      <Path d="M15 2H9a1 1 0 0 0-1 1v2a1 1 0 0 0 1 1h6a1 1 0 0 0 1-1V3a1 1 0 0 0-1-1Z" />
    </Svg>
  );
}

function CheckIcon() {
  return (
    <Svg width={18} height={18} viewBox="0 0 24 24" fill="none" stroke="#34D399" strokeWidth={2.5} strokeLinecap="round" strokeLinejoin="round">
      <Path d="M20 6L9 17l-5-5" />
    </Svg>
  );
}

export function TelegramLinkModal({ visible, onDismiss, onConnected }: TelegramLinkModalProps) {
  const { colors } = useTheme();
  const styles = useMemo(() => createStyles(colors), [colors]);
  const [linkInfo, setLinkInfo] = useState<TelegramLinkInfo | null>(null);
  const [isLoading, setIsLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);
  const [connected, setConnected] = useState(false);
  const [expired, setExpired] = useState(false);
  const pollRef = useRef<ReturnType<typeof setInterval> | null>(null);
  const timeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  const cleanup = useCallback(() => {
    if (pollRef.current) clearInterval(pollRef.current);
    if (timeoutRef.current) clearTimeout(timeoutRef.current);
    pollRef.current = null;
    timeoutRef.current = null;
  }, []);

  const reset = useCallback(() => {
    cleanup();
    setLinkInfo(null);
    setIsLoading(false);
    setError(null);
    setCopied(false);
    setConnected(false);
    setExpired(false);
  }, [cleanup]);

  const fetchLink = useCallback(async () => {
    setIsLoading(true);
    setError(null);
    setExpired(false);
    setCopied(false);
    setConnected(false);
    try {
      const info = await mobileApi.telegramLink();
      setLinkInfo(info);
    } catch (e) {
      setError(e instanceof Error ? e.message : "Failed to generate link code");
    } finally {
      setIsLoading(false);
    }
  }, []);

  // Fetch link code when modal opens
  useEffect(() => {
    if (visible) {
      reset();
      void fetchLink();
    } else {
      reset();
    }
  }, [visible, fetchLink, reset]);

  // Start polling + timeout when we have a link code
  useEffect(() => {
    if (!visible || !linkInfo || connected || expired) return;

    pollRef.current = setInterval(async () => {
      try {
        const settings = await mobileApi.getNotificationSettings();
        if (settings.telegram_connected) {
          cleanup();
          setConnected(true);
        }
      } catch {
        // Ignore poll errors
      }
    }, POLL_INTERVAL_MS);

    timeoutRef.current = setTimeout(() => {
      cleanup();
      setExpired(true);
    }, POLL_TIMEOUT_MS);

    return cleanup;
  }, [visible, linkInfo, connected, expired, cleanup]);

  // Auto-dismiss after connection
  useEffect(() => {
    if (connected) {
      const timer = setTimeout(() => onConnected(), 1200);
      return () => clearTimeout(timer);
    }
  }, [connected, onConnected]);

  const handleOpenTelegram = useCallback(async () => {
    if (!linkInfo) return;
    const tgUrl = `tg://resolve?domain=${linkInfo.bot_username}&start=${linkInfo.link_code}`;
    const webUrl = `https://t.me/${linkInfo.bot_username}?start=${linkInfo.link_code}`;

    try {
      const canOpen = await Linking.canOpenURL(tgUrl);
      await Linking.openURL(canOpen ? tgUrl : webUrl);
    } catch {
      try {
        await Linking.openURL(webUrl);
      } catch {
        setError("Could not open Telegram");
      }
    }
  }, [linkInfo]);

  const handleCopy = useCallback(async () => {
    if (!linkInfo) return;
    await Clipboard.setStringAsync(`/start ${linkInfo.link_code}`);
    setCopied(true);
    setTimeout(() => setCopied(false), 2000);
  }, [linkInfo]);

  return (
    <Modal visible={visible} transparent animationType="fade" statusBarTranslucent onRequestClose={onDismiss}>
      <View style={styles.backdrop}>
        <View style={styles.card}>
          {/* Header */}
          <Text style={styles.title}>Connect Telegram</Text>
          <Text style={styles.subtitle}>
            Receive approval notifications directly in Telegram.
          </Text>

          {/* Loading state */}
          {isLoading && (
            <View style={styles.centerWrap}>
              <ActivityIndicator size="small" color={colors.primary} />
            </View>
          )}

          {/* Error state */}
          {error && (
            <View style={styles.centerWrap}>
              <Text style={styles.errorText}>{error}</Text>
              <Pressable style={styles.retryBtn} onPress={() => void fetchLink()}>
                <Text style={styles.retryText}>Retry</Text>
              </Pressable>
            </View>
          )}

          {/* Connected state */}
          {connected && (
            <View style={styles.centerWrap}>
              <CheckIcon />
              <Text style={styles.successText}>Telegram connected!</Text>
            </View>
          )}

          {/* Expired state */}
          {expired && !connected && (
            <View style={styles.centerWrap}>
              <Text style={styles.errorText}>Link code expired</Text>
              <Pressable style={styles.retryBtn} onPress={() => void fetchLink()}>
                <Text style={styles.retryText}>Generate New Code</Text>
              </Pressable>
            </View>
          )}

          {/* Active link state */}
          {linkInfo && !isLoading && !error && !connected && !expired && (
            <>
              {/* Open in Telegram button */}
              <Pressable style={styles.telegramBtn} onPress={handleOpenTelegram}>
                <TelegramIcon />
                <Text style={styles.telegramBtnText}>Open in Telegram</Text>
              </Pressable>

              {/* Divider */}
              <View style={styles.divider}>
                <View style={styles.dividerLine} />
                <Text style={styles.dividerText}>or copy the code</Text>
                <View style={styles.dividerLine} />
              </View>

              {/* Code display + copy */}
              <View style={styles.codeRow}>
                <Text style={styles.codeText} selectable>/start {linkInfo.link_code}</Text>
                <Pressable style={styles.copyBtn} onPress={() => void handleCopy()}>
                  {copied ? <CheckIcon /> : <CopyIcon color={colors.textSecondary} />}
                </Pressable>
              </View>

              <Text style={styles.instructionText}>
                Send this to @{linkInfo.bot_username} on Telegram
              </Text>

              {/* Polling indicator */}
              <View style={styles.waitingRow}>
                <ActivityIndicator size="small" color={colors.textMuted} />
                <Text style={styles.waitingText}>Waiting for connection...</Text>
              </View>
            </>
          )}

          {/* Cancel button */}
          {!connected && (
            <Pressable style={styles.cancelBtn} onPress={onDismiss}>
              <Text style={styles.cancelText}>Cancel</Text>
            </Pressable>
          )}
        </View>
      </View>
    </Modal>
  );
}

const createStyles = (c: ThemeColors) =>
  StyleSheet.create({
    backdrop: {
      flex: 1,
      backgroundColor: "rgba(0,0,0,0.65)",
      justifyContent: "center",
      alignItems: "center",
      paddingHorizontal: 28,
    },
    card: {
      width: "100%",
      maxWidth: 320,
      backgroundColor: c.card,
      borderRadius: radius.lg,
      borderWidth: 1,
      borderColor: c.border,
      padding: spacing.xxl,
      gap: spacing.lg,
    },
    title: {
      ...typeScale.h2,
      color: c.textPrimary,
      textAlign: "center",
    },
    subtitle: {
      ...typeScale.description,
      color: c.textSecondary,
      textAlign: "center",
    },
    centerWrap: {
      alignItems: "center",
      gap: spacing.sm,
      paddingVertical: spacing.md,
    },
    errorText: {
      ...typeScale.body,
      color: c.danger,
      textAlign: "center",
    },
    successText: {
      ...typeScale.label,
      color: c.success,
      textAlign: "center",
    },
    retryBtn: {
      paddingHorizontal: spacing.lg,
      minHeight: TOUCH_TARGET,
      borderRadius: radius.md,
      borderWidth: 1,
      borderColor: c.border,
      alignItems: "center",
      justifyContent: "center",
    },
    retryText: {
      ...typeScale.label,
      color: c.textSecondary,
    },
    telegramBtn: {
      flexDirection: "row",
      alignItems: "center",
      justifyContent: "center",
      gap: spacing.sm,
      minHeight: TOUCH_TARGET,
      borderRadius: radius.md,
      backgroundColor: "#229ED9",
    },
    telegramBtnText: {
      ...typeScale.label,
      color: c.onPrimary,
    },
    divider: {
      flexDirection: "row",
      alignItems: "center",
      gap: spacing.sm,
    },
    dividerLine: {
      flex: 1,
      height: 1,
      backgroundColor: c.borderSoft,
    },
    dividerText: {
      ...typeScale.overline,
      color: c.textMuted,
      letterSpacing: 0.6,
    },
    codeRow: {
      flexDirection: "row",
      alignItems: "center",
      gap: spacing.sm,
      paddingVertical: spacing.md,
      paddingHorizontal: spacing.lg,
      borderRadius: radius.md,
      borderWidth: 1,
      borderColor: c.border,
      backgroundColor: c.cardSoft,
    },
    codeText: {
      flex: 1,
      ...typeScale.mono,
      color: c.textPrimary,
    },
    copyBtn: {
      padding: spacing.xs,
    },
    instructionText: {
      ...typeScale.small,
      color: c.textMuted,
      textAlign: "center",
    },
    waitingRow: {
      flexDirection: "row",
      alignItems: "center",
      justifyContent: "center",
      gap: spacing.sm,
    },
    waitingText: {
      ...typeScale.small,
      color: c.textMuted,
    },
    cancelBtn: {
      alignItems: "center",
      minHeight: TOUCH_TARGET,
      justifyContent: "center",
    },
    cancelText: {
      ...typeScale.label,
      color: c.textMuted,
    },
  });
