import { useEffect, useMemo, useState } from "react";
import {
  Alert,
  Linking,
  Pressable,
  StyleSheet,
  Switch,
  View,
} from "react-native";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Bell, Send } from "lucide-react-native";
import type { SheetScreenProps } from "../../app/navigationTypes";
import { DynamicTypeText as Text } from "../../components/DynamicTypeText";
import { OfflineBanner } from "../../components/OfflineBanner";
import { SkeletonRows } from "../../components/Skeleton";
import { TelegramLinkModal } from "../../components/TelegramLinkModal";
import {
  ToastOverlay,
  type ToastKind,
  type ToastState,
} from "../../components/ToastOverlay";
import {
  SettingsGroup,
  SettingsMenuRow,
  SettingsPage,
  SettingsRow,
} from "../../components/settings/SettingsList";
import { useNetworkStatus } from "../../hooks/useNetworkStatus";
import { mobileApi } from "../../lib/api/mobileApi";
import { resolveErrorMessage } from "../../lib/api/errorMessages";
import { activatePushAfterLogin } from "../../lib/notifications/pushNotifications";
import { capture } from "../../lib/telemetry";
import { useTheme } from "../../theme/ThemeContext";
import type { ThemeColors } from "../../theme/mobileTheme";
import { radius, spacing, typeScale } from "../../theme/designTokens";
import { approvalDeliveryWarning } from "./approvalDeliveryPolicy";

// Push-register failure modes are surfaced as a tagged error so the
// caller can branch on `reason` (e.g. iOS denial → Alert + Linking
// to system Settings) instead of brittle string-matching the message.
class PushRegisterError extends Error {
  constructor(
    public readonly reason:
      "permission_denied" | "token_unavailable" | "register_failed",
    message: string,
  ) {
    super(message);
    this.name = "PushRegisterError";
  }
}

const APPROVAL_MODES = [
  ["writes_only", "Writes only (default)"],
  ["all", "All requests"],
  ["off", "Off"],
] as const;

/** Settings › Notifications: delivery channels and the approval default. */
export function NotificationSettingsScreen(
  _props: SheetScreenProps<"NotificationSettings">,
) {
  const { colors } = useTheme();
  const styles = useMemo(() => createStyles(colors), [colors]);
  const queryClient = useQueryClient();
  const { isConnected, recheckConnection } = useNetworkStatus();
  const isOffline = !isConnected;
  const [toast, setToast] = useState<ToastState | null>(null);
  const showToast = (message: string, kind: ToastKind) =>
    setToast({ message, kind });
  useEffect(() => {
    if (!toast) return;
    const timer = setTimeout(() => setToast(null), 2400);
    return () => clearTimeout(timer);
  }, [toast]);

  const {
    data: notifSettings,
    isLoading: isNotifLoading,
    isError: isNotifError,
    refetch: refetchNotifSettings,
  } = useQuery({
    queryKey: ["account", "notificationSettings"],
    queryFn: () => mobileApi.getNotificationSettings(),
  });

  const selectedApprovalMode =
    notifSettings?.global_approval_mode ??
    (notifSettings?.approval_required ? "all" : "off");

  const notifMutation = useMutation({
    mutationFn: (
      payload: Parameters<typeof mobileApi.updateNotificationSettings>[0],
    ) => mobileApi.updateNotificationSettings(payload),
    onSuccess: () => {
      void queryClient.invalidateQueries({
        queryKey: ["account", "notificationSettings"],
      });
      showToast("Notification settings updated", "success");
    },
    onError: (error) => {
      showToast(resolveErrorMessage(error), "error");
      void refetchNotifSettings();
    },
  });

  const [isTelegramLinkVisible, setIsTelegramLinkVisible] = useState(false);

  // Manual re-registration when the device isn't on the user's account
  // (permission denied at first launch, token fetch failed, or original
  // post-login registration silently failed). Surfaces the actual reason
  // so the user can act — `permission_denied` lifts them to OS settings,
  // others fall back to a toast with a hint.
  const pushRegisterMutation = useMutation({
    mutationFn: async () => {
      const result = await activatePushAfterLogin({ forceRegister: true });
      if (!result.registered) {
        if (result.reason === "permission_denied") {
          throw new PushRegisterError(
            "permission_denied",
            "Notification permission is off for NyxID.",
          );
        }
        if (result.reason === "token_unavailable") {
          throw new PushRegisterError(
            "token_unavailable",
            "This device can't get a push token. Try a real device or reinstall.",
          );
        }
        throw new PushRegisterError(
          "register_failed",
          "Couldn't register this device. Check your connection and try again.",
        );
      }
      return result;
    },
    onSuccess: () => {
      void queryClient.invalidateQueries({
        queryKey: ["account", "notificationSettings"],
      });
      showToast("Push enabled on this device", "success");
    },
    onError: (error) => {
      if (
        error instanceof PushRegisterError &&
        error.reason === "permission_denied"
      ) {
        // Linking.openSettings() opens this app's settings page directly
        // on iOS (and Android). Guarded by an Alert so the user opts in
        // instead of being yanked out of the app on a tap.
        Alert.alert(
          "Enable Notifications",
          "Push notifications are off for NyxID. Open Settings to enable them?",
          [
            { text: "Not now", style: "cancel" },
            {
              text: "Open Settings",
              onPress: () => {
                Linking.openSettings().catch(() => {
                  showToast(
                    "Couldn't open Settings — open it manually.",
                    "error",
                  );
                });
              },
            },
          ],
        );
        return;
      }
      showToast(resolveErrorMessage(error), "error");
    },
  });

  const telegramDisconnectMutation = useMutation({
    mutationFn: () => mobileApi.telegramDisconnect(),
    onSuccess: () => {
      void queryClient.invalidateQueries({
        queryKey: ["account", "notificationSettings"],
      });
      showToast("Telegram disconnected", "success");
    },
    onError: (error) => {
      showToast(resolveErrorMessage(error), "error");
    },
  });

  /** True when the other channel can't receive notifications */
  const isLastChannel = (turning: "push" | "telegram"): boolean => {
    if (!notifSettings) return false;
    if (turning === "push") {
      return !(
        notifSettings.telegram_connected && notifSettings.telegram_enabled
      );
    }
    return !(notifSettings.push_enabled && notifSettings.push_device_count > 0);
  };

  const handleTelegramConnected = () => {
    setIsTelegramLinkVisible(false);
    void queryClient.invalidateQueries({
      queryKey: ["account", "notificationSettings"],
    });
    showToast("Telegram connected!", "success");
  };

  // Helper: emit `ui.mobile_preference_toggled` only after a settings
  // mutation actually succeeds on the server. Attached as a per-call
  // `onSuccess` so the shared `notifMutation` can fire different events
  // based on which setting was toggled. Failed / offline / rejected
  // updates never count as completed toggles.
  const emitPreferenceToggledOnSuccess = (
    prefName: "push_enabled" | "telegram_enabled",
    value: boolean,
  ) => ({
    onSuccess: () => {
      capture({
        name: "ui.mobile_preference_toggled",
        props: { name: prefName, value },
      });
    },
  });

  const handleTogglePush = (newValue: boolean) => {
    if (newValue) {
      // Enable path has no confirmation -- user tap IS the commit. Emit
      // only after the server accepts the setting change.
      notifMutation.mutate(
        { push_enabled: true },
        emitPreferenceToggledOnSuccess("push_enabled", true),
      );
      return;
    }
    const willUseWebApproval =
      isLastChannel("push") && notifSettings?.approval_required;
    const message = willUseWebApproval
      ? approvalDeliveryWarning(notifSettings)
      : "You will no longer receive push notifications for approval requests.";

    Alert.alert("Disable Push Notifications", message, [
      { text: "Cancel", style: "cancel" },
      {
        text: "Disable",
        style: "destructive",
        onPress: () => {
          // Emit only on mutation success so a failed update (offline /
          // rejected / network error) doesn't count as a completed
          // disable in analytics. Cancel naturally skips this arm.
          notifMutation.mutate(
            {
              push_enabled: false,
            },
            emitPreferenceToggledOnSuccess("push_enabled", false),
          );
        },
      },
    ]);
  };

  const handleToggleTelegram = (newValue: boolean) => {
    if (newValue) {
      notifMutation.mutate(
        { telegram_enabled: true },
        emitPreferenceToggledOnSuccess("telegram_enabled", true),
      );
      return;
    }
    const willUseWebApproval =
      isLastChannel("telegram") && notifSettings?.approval_required;
    const message = willUseWebApproval
      ? `This disconnects Telegram. ${approvalDeliveryWarning(notifSettings)}`
      : "This will disconnect your Telegram account and stop all Telegram notifications.";

    Alert.alert("Disable Telegram", message, [
      { text: "Cancel", style: "cancel" },
      {
        text: "Disable",
        style: "destructive",
        onPress: async () => {
          // Disable notifications in one call, then disconnect. Only emit after BOTH land -- the earlier design
          // emitted before the settings update, which overcounted
          // completed toggles on offline/error.
          //
          // Partial-success guard: if settings update succeeded but
          // disconnect fails, the backend has already flipped
          // `telegram_enabled=false`. The UI MUST invalidate the
          // cached settings query so the toggle reflects the new
          // state; otherwise the screen would keep showing "enabled"
          // until the user manually refreshes.
          let settingsOk = false;
          try {
            await mobileApi.updateNotificationSettings({
              telegram_enabled: false,
            });
            settingsOk = true;
          } catch {
            // disconnect will cascade anyway; telemetry won't fire.
          }
          telegramDisconnectMutation.mutate(undefined, {
            onSuccess: () => {
              if (settingsOk) {
                capture({
                  name: "ui.mobile_preference_toggled",
                  props: { name: "telegram_enabled", value: false },
                });
              }
            },
            onSettled: () => {
              // Always invalidate, even on partial success: if settings
              // succeeded but disconnect failed, the UI would otherwise
              // keep showing the old (enabled) state.
              if (settingsOk) {
                void queryClient.invalidateQueries({
                  queryKey: ["account", "notificationSettings"],
                });
              }
            },
          });
        },
      },
    ]);
  };

  const switchTrack = { false: colors.borderSoft, true: colors.success };

  return (
    <>
      <SettingsPage>
        {isOffline && (
          <OfflineBanner
            subtitle="Notification settings need a connection"
            onRetry={async () => {
              const online = await recheckConnection();
              if (online) void refetchNotifSettings();
              else
                showToast("Still offline — will retry when connected", "error");
            }}
          />
        )}
        {isOffline ? (
          <Text style={styles.note}>Requires network connection</Text>
        ) : isNotifLoading || (!notifSettings && !isNotifError) ? (
          <SkeletonRows
            rows={2}
            title
            accessibilityLabel="Loading notification settings"
          />
        ) : !notifSettings ? (
          // No error UI yet: a failed first load keeps its old placeholder.
          <Text style={styles.note}>Loading...</Text>
        ) : (
          <>
            <SettingsGroup
              title="Delivery"
              footer="Approval requests and task results arrive on these channels."
            >
              <SettingsRow
                icon={Bell}
                label="Push Notifications"
                accessory={
                  notifSettings.push_device_count > 0 ? (
                    <Switch
                      accessibilityLabel="Push Notifications"
                      value={notifSettings.push_enabled}
                      onValueChange={handleTogglePush}
                      disabled={notifMutation.isPending}
                      trackColor={switchTrack}
                    />
                  ) : (
                    <Pressable
                      accessibilityRole="button"
                      onPress={() => pushRegisterMutation.mutate()}
                      disabled={pushRegisterMutation.isPending}
                      style={styles.pill}
                    >
                      <Text style={styles.pillText}>
                        {pushRegisterMutation.isPending
                          ? "Enabling…"
                          : "Enable on this device"}
                      </Text>
                    </Pressable>
                  )
                }
              />
              {notifSettings.telegram_connected ? (
                <SettingsRow
                  icon={Send}
                  label="Telegram"
                  detail={`@${notifSettings.telegram_username ?? "linked"}`}
                  accessory={
                    <Switch
                      accessibilityLabel="Telegram"
                      value={notifSettings.telegram_enabled}
                      onValueChange={handleToggleTelegram}
                      disabled={notifMutation.isPending}
                      trackColor={switchTrack}
                    />
                  }
                />
              ) : (
                <SettingsRow
                  icon={Send}
                  label="Telegram"
                  value="Link account"
                  onPress={() => {
                    capture({
                      name: "ui.mobile_dialog_opened",
                      props: {
                        dialog_id: "other",
                        entry_point: "account_settings.telegram_link",
                      },
                    });
                    setIsTelegramLinkVisible(true);
                  }}
                />
              )}
            </SettingsGroup>
            <SettingsGroup
              title="Approval requests"
              inset={false}
              footer={
                <View style={styles.footer}>
                  <Text style={styles.hint}>
                    Per-service and organization policies still apply.
                  </Text>
                  <Text style={styles.hint}>
                    {notifSettings.approval_required
                      ? approvalDeliveryWarning(notifSettings)
                      : "Push or Telegram delivers requests when approval protection is on."}
                  </Text>
                </View>
              }
            >
              <SettingsMenuRow
                label="Request approval for"
                value={selectedApprovalMode}
                options={APPROVAL_MODES.map(([value, label]) => ({
                  value,
                  label,
                }))}
                disabled={notifMutation.isPending}
                onChange={(mode) =>
                  notifMutation.mutate({ global_approval_mode: mode })
                }
              />
            </SettingsGroup>
          </>
        )}
      </SettingsPage>
      <TelegramLinkModal
        visible={isTelegramLinkVisible}
        onDismiss={() => setIsTelegramLinkVisible(false)}
        onConnected={handleTelegramConnected}
      />
      <ToastOverlay toast={toast} />
    </>
  );
}

const createStyles = (c: ThemeColors) =>
  StyleSheet.create({
    note: {
      ...typeScale.caption,
      color: c.textMuted,
      textAlign: "center",
      paddingVertical: spacing.md,
    },
    pill: {
      minHeight: 44,
      maxWidth: "100%",
      flexShrink: 1,
      justifyContent: "center",
      backgroundColor: c.primaryTone.bg,
      borderWidth: 1,
      borderColor: c.primaryTone.border,
      borderRadius: radius.sm,
      paddingHorizontal: spacing.sm,
      paddingVertical: 2,
    },
    pillText: { flexShrink: 1, ...typeScale.badge, color: c.primaryOnTint },
    footer: { gap: spacing.xs, paddingHorizontal: spacing.xxl },
    hint: { ...typeScale.small, color: c.textMuted },
  });
