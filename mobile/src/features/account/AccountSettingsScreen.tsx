import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { NativeStackScreenProps } from "@react-navigation/native-stack";
import { Alert, Image, Linking, Pressable, ScrollView, StyleSheet, Switch, Text, View } from "react-native";
import Svg, { Circle, Path } from "react-native-svg";
import { RootStackParamList } from "../../app/AppNavigator";

import { PrimaryButton } from "../../components/PrimaryButton";
import { ScreenContainer } from "../../components/ScreenContainer";
import { OfflineBanner } from "../../components/OfflineBanner";
import { TelegramLinkModal } from "../../components/TelegramLinkModal";
import { ToastKind, ToastOverlay, ToastState } from "../../components/ToastOverlay";
import { useAuthSession } from "../auth/AuthSessionContext";
import { capture } from "../../lib/telemetry";
import { useNetworkStatus } from "../../hooks/useNetworkStatus";
import { mobileApi } from "../../lib/api/mobileApi";
import { isApiError } from "../../lib/api/ApiError";
import { resolveErrorMessage } from "../../lib/api/errorMessages";
import { activatePushAfterLogin } from "../../lib/notifications/pushNotifications";
import { useTheme } from "../../theme/ThemeContext";
import type { ThemeColors } from "../../theme/mobileTheme";
import { createFlowStyles } from "../../theme/flowStyles";
import { BOTTOM_NAV_CLEARANCE, radius, spacing, typeScale } from "../../theme/designTokens";
import { useEffect, useMemo, useState } from "react";

type Props = NativeStackScreenProps<RootStackParamList, "AccountSettings">;

// Push-register failure modes are surfaced as a tagged error so the
// caller can branch on `reason` (e.g. iOS denial → Alert + Linking
// to system Settings) instead of brittle string-matching the message.
class PushRegisterError extends Error {
  constructor(
    public readonly reason: "permission_denied" | "token_unavailable" | "register_failed",
    message: string,
  ) {
    super(message);
    this.name = "PushRegisterError";
  }
}

function resolveDeleteAccountError(error: unknown): {
  message: string;
  shouldForceSignOut: boolean;
} {
  // Use errorKey for reliable matching when available (machine-readable, stable)
  if (isApiError(error)) {
    const key = error.errorKey;
    if (key === "unauthorized" || key === "authentication_failed" || key === "token_expired") {
      return { message: "Session expired. Please sign in again.", shouldForceSignOut: true };
    }
    if (key === "not_found" || error.statusCode === 404) {
      return { message: "Account not found or already deleted.", shouldForceSignOut: true };
    }
  }

  const raw = error instanceof Error ? error.message : "";
  const lower = raw.toLowerCase();

  if (lower.includes("auth_session_missing") || lower.includes("request_failed_401")) {
    return { message: "Session expired. Please sign in again.", shouldForceSignOut: true };
  }

  if (lower.includes("network request failed") || lower.includes("failed to fetch")) {
    return { message: "Network error. Check API server and try again.", shouldForceSignOut: false };
  }

  const fallback = __DEV__ && raw ? raw : "Failed to delete account. Please try again.";
  return { message: fallback, shouldForceSignOut: false };
}

function AccountRow({
  label,
  value,
  isLast,
  onPress,
}: {
  label: string;
  value?: string;
  isLast?: boolean;
  onPress?: () => void;
}) {
  const { colors } = useTheme();
  const styles = useMemo(() => createStyles(colors), [colors]);
  const content = (
    <View style={[styles.accountRow, isLast && styles.accountRowLast]}>
      <Text style={styles.accountRowLabel}>{label}</Text>
      <View style={styles.accountRowRight}>
        {value ? <Text style={styles.accountRowValue}>{value}</Text> : null}
        {onPress ? <Text style={styles.accountRowArrow}>→</Text> : null}
      </View>
    </View>
  );
  if (onPress) {
    return <Pressable onPress={onPress}>{content}</Pressable>;
  }
  return content;
}

function getInitials(name?: string | null, email?: string): string {
  const trimmedName = name?.trim();
  if (trimmedName) {
    return trimmedName
      .split(" ")
      .map((w) => w[0])
      .join("")
      .toUpperCase()
      .slice(0, 2);
  }
  return (email?.[0] ?? "?").toUpperCase();
}

function getIdentityName(name?: string | null, provider?: string | null): string {
  const trimmedName = name?.trim();
  if (trimmedName) return trimmedName;
  if (provider === "apple") return "Apple account";
  return "User";
}

function formatSignInMethod(provider?: string | null): string {
  if (!provider) return "Email/Password";
  switch (provider.toLowerCase()) {
    case "google":
      return "Google";
    case "github":
      return "GitHub";
    case "apple":
      return "Apple";
    default:
      return provider.charAt(0).toUpperCase() + provider.slice(1);
  }
}

function getDisplayNameValue(name?: string | null, provider?: string | null): string {
  const trimmedName = name?.trim();
  if (trimmedName) return trimmedName;
  if (provider === "apple") return "Not provided by Apple";
  return "No display name provided";
}

export function AccountSettingsScreen({ navigation }: Props) {
  const { colors, mode, preference, setPreference } = useTheme();
  const styles = useMemo(() => createStyles(colors), [colors]);
  const flowStyles = useMemo(() => createFlowStyles(colors), [colors]);
  const [toast, setToast] = useState<ToastState | null>(null);
  const queryClient = useQueryClient();
  const { signOut } = useAuthSession();
  const { isConnected, recheckConnection } = useNetworkStatus();

  const {
    data: profile,
    isLoading: isProfileLoading,
    isError: isProfileError,
    error: profileError,
    refetch: refetchProfile,
  } = useQuery({
    queryKey: ["account", "profile"],
    queryFn: () => mobileApi.getAccountProfile(),
  });

  const showToast = (message: string, kind: ToastKind) => setToast({ message, kind });

  useEffect(() => {
    if (!toast) return;
    const timer = setTimeout(() => setToast(null), 2400);
    return () => clearTimeout(timer);
  }, [toast]);

  const {
    data: notifSettings,
    isLoading: isNotifLoading,
    refetch: refetchNotifSettings,
  } = useQuery({
    queryKey: ["account", "notificationSettings"],
    queryFn: () => mobileApi.getNotificationSettings(),
  });

  const notifMutation = useMutation({
    mutationFn: (payload: Parameters<typeof mobileApi.updateNotificationSettings>[0]) =>
      mobileApi.updateNotificationSettings(payload),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ["account", "notificationSettings"] });
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
      void queryClient.invalidateQueries({ queryKey: ["account", "notificationSettings"] });
      showToast("Push enabled on this device", "success");
    },
    onError: (error) => {
      if (error instanceof PushRegisterError && error.reason === "permission_denied") {
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
                  showToast("Couldn't open Settings — open it manually.", "error");
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
      void queryClient.invalidateQueries({ queryKey: ["account", "notificationSettings"] });
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
      return !(notifSettings.telegram_connected && notifSettings.telegram_enabled);
    }
    return !(notifSettings.push_enabled && notifSettings.push_device_count > 0);
  };

  const handleTelegramConnected = () => {
    setIsTelegramLinkVisible(false);
    void queryClient.invalidateQueries({ queryKey: ["account", "notificationSettings"] });
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
    const willDisableApproval = isLastChannel("push") && notifSettings?.approval_required;
    const message = willDisableApproval
      ? "This is your only notification channel. Disabling it will also turn off approval protection."
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
              ...(willDisableApproval && { approval_required: false }),
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
    const willDisableApproval = isLastChannel("telegram") && notifSettings?.approval_required;
    const willDisconnect = true; // disabling always disconnects
    const message = willDisableApproval
      ? "This is your only notification channel. Disabling it will disconnect Telegram and turn off approval protection."
      : "This will disconnect your Telegram account and stop all Telegram notifications.";

    Alert.alert("Disable Telegram", message, [
      { text: "Cancel", style: "cancel" },
      {
        text: "Disable",
        style: "destructive",
        onPress: async () => {
          // Disable notifications + approval in one call, then
          // disconnect. Only emit after BOTH land -- the earlier design
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
              ...(willDisableApproval && { approval_required: false }),
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

  const deleteAccountMutation = useMutation({
    mutationFn: () => mobileApi.deleteAccount(),
    onSuccess: async () => {
      showToast("Account deleted. You have been signed out.", "success");
      try { await signOut(); } catch {}
      queryClient.clear();
    },
    onError: (error) => {
      const resolved = resolveDeleteAccountError(error);
      showToast(resolved.message, "error");
      if (resolved.shouldForceSignOut) {
        void signOut().then(() => queryClient.clear()).catch(() => {});
      }
    },
  });

  const handleSignOut = () => {
    Alert.alert("Sign Out", "Do you want to sign out from this account?", [
      { text: "Cancel", style: "cancel" },
      {
        text: "Sign Out",
        style: "destructive",
        onPress: () => {
          showToast("You are signed out.", "info");
          void signOut().then(() => queryClient.clear()).catch(() => {});
        },
      },
    ]);
  };

  const handleDeleteAccount = () => {
    // Opening a native confirm counts as opening the
    // delete_account_confirm dialog. If the user cancels, the dialog
    // ends on step 1 without an abandonment event (native Alert hides
    // its lifecycle from us); the confirm path emits destructive_confirmed.
    capture({
      name: "ui.mobile_dialog_opened",
      props: { dialog_id: "delete_account_confirm", entry_point: "account_settings" },
    });
    Alert.alert(
      "Delete Account",
      "This action is permanent and will permanently delete your account and server-side data.",
      [
        { text: "Cancel", style: "cancel" },
        {
          text: "Delete",
          style: "destructive",
          onPress: () => {
            capture({
              name: "ui.mobile_destructive_confirmed",
              props: { domain: "account", action: "delete_account" },
            });
            setToast(null);
            deleteAccountMutation.mutate();
          },
        },
      ]
    );
  };

  const initials = getInitials(profile?.display_name, profile?.email);
  const identityName = getIdentityName(profile?.display_name, profile?.social_provider);
  const isOffline = !isConnected;
  const profileOpacity = isOffline ? 0.5 : 1;

  return (
    <ScreenContainer>
      <ScrollView
        style={flowStyles.content}
        contentContainerStyle={[flowStyles.scrollContent, { paddingHorizontal: spacing.xxl, paddingBottom: BOTTOM_NAV_CLEARANCE }]}
        showsVerticalScrollIndicator={false}
      >
        {isOffline && <OfflineBanner subtitle="Some features unavailable" onRetry={async () => {
          const online = await recheckConnection();
          if (online) {
            void refetchProfile();
            void refetchNotifSettings();
          } else {
            showToast("Still offline — will retry when connected", "error");
          }
        }} />}

        {/* User identity header */}
        <View style={styles.identityHeader}>
          <View style={styles.avatarCircle}>
            {profile?.avatar_url ? (
              <Image source={{ uri: profile.avatar_url }} style={styles.avatarImage} />
            ) : (
              <Text style={styles.avatarText}>{initials}</Text>
            )}
          </View>
          <View style={styles.identityInfo}>
            <Text style={styles.identityName}>{identityName}</Text>
            <Text style={styles.identityEmail}>{profile?.email ?? "..."}</Text>
          </View>
          <View style={[styles.statusBadge, isOffline && styles.statusBadgeOffline]}>
            <Text style={[styles.statusBadgeText, isOffline && styles.statusBadgeTextOffline]}>
              {isOffline ? "OFFLINE" : "ACTIVE"}
            </Text>
          </View>
        </View>

        {/* Profile card */}
        <View style={[styles.card, { opacity: profileOpacity }]}>
          <Text style={styles.cardTitle}>Profile</Text>
          {isProfileLoading ? (
            <Text style={styles.metaText}>Loading...</Text>
          ) : isProfileError || !profile ? (
            <>
              <Text style={styles.errorText}>Failed to load profile</Text>
              <PrimaryButton label="Retry" kind="ghost" onPress={() => refetchProfile()} />
            </>
          ) : (
            <>
              <AccountRow
                label="Display Name"
                value={getDisplayNameValue(profile.display_name, profile.social_provider)}
              />
              <AccountRow label="Email" value={profile.email} />
              <AccountRow label="Sign-in Method" value={formatSignInMethod(profile.social_provider)} isLast />
            </>
          )}
        </View>

        {/* Notifications card */}
        <View style={[styles.card, isOffline && { opacity: 0.35 }]}>
          <Text style={styles.cardTitle}>Notifications</Text>
          {isOffline ? (
            <Text style={styles.offlineNote}>Requires network connection</Text>
          ) : isNotifLoading ? (
            <Text style={styles.metaText}>Loading...</Text>
          ) : (
            <>
              <View style={styles.accountRow}>
                <Text style={styles.accountRowLabel}>Push Notifications</Text>
                <View style={styles.accountRowRight}>
                  {notifSettings && notifSettings.push_device_count > 0 ? (
                    <Switch
                      value={notifSettings.push_enabled}
                      onValueChange={handleTogglePush}
                      disabled={notifMutation.isPending}
                      trackColor={{ false: colors.borderSoft, true: colors.success }}
                    />
                  ) : (
                    <Pressable
                      onPress={() => pushRegisterMutation.mutate()}
                      disabled={pushRegisterMutation.isPending}
                      style={styles.linkPill}
                    >
                      <Text style={styles.linkPillText}>
                        {pushRegisterMutation.isPending ? "Enabling…" : "Enable on this device"}
                      </Text>
                    </Pressable>
                  )}
                </View>
              </View>
              <View style={[styles.accountRow, styles.accountRowLast]}>
                {notifSettings?.telegram_connected ? (
                  <View style={styles.channelRowLeft}>
                    <Text style={styles.accountRowLabel}>Telegram</Text>
                    <View style={styles.connectedPill}>
                      <Text style={styles.connectedPillText}>@{notifSettings.telegram_username ?? "linked"}</Text>
                    </View>
                  </View>
                ) : (
                  <Pressable
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
                    style={styles.channelRowLeft}
                  >
                    <Text style={styles.accountRowLabel}>Telegram</Text>
                    <View style={styles.linkPill}>
                      <Text style={styles.linkPillText}>Link account</Text>
                    </View>
                  </Pressable>
                )}
                {notifSettings?.telegram_connected && (
                  <Switch
                    value={notifSettings.telegram_enabled}
                    onValueChange={handleToggleTelegram}
                    disabled={notifMutation.isPending}
                    trackColor={{ false: colors.borderSoft, true: colors.success }}
                  />
                )}
              </View>
              {notifSettings && (
                <Text style={styles.channelHint}>
                  Either Push or Telegram must stay enabled to receive approval requests.
                </Text>
              )}
            </>
          )}
        </View>

        {/* Appearance card */}
        <View style={styles.card}>
          <Text style={styles.cardTitle}>Appearance</Text>
          <View style={styles.themeToggleWrap}>
            {(() => {
              const isLight = preference === "light" || (preference === "system" && mode === "light");
              const isDark = preference === "dark" || (preference === "system" && mode === "dark");
              const lightColor = isLight ? colors.primary : colors.textMuted;
              const darkColor = isDark ? colors.primary : colors.textMuted;
              return (
                <>
                  <Pressable
                    style={[styles.themeToggleHalf, isLight && styles.themeToggleHalfActive]}
                    onPress={() => {
                      capture({
                        name: "ui.mobile_preference_toggled",
                        props: { name: "theme", value: "light" },
                      });
                      setPreference("light");
                    }}
                  >
                    <Svg width={16} height={16} viewBox="0 0 24 24" fill="none" stroke={lightColor} strokeWidth={2} strokeLinecap="round" strokeLinejoin="round">
                      <Circle cx={12} cy={12} r={5} />
                      <Path d="M12 1v2M12 21v2M4.22 4.22l1.42 1.42M18.36 18.36l1.42 1.42M1 12h2M21 12h2M4.22 19.78l1.42-1.42M18.36 5.64l1.42-1.42" />
                    </Svg>
                    <Text style={[styles.themeToggleLabel, isLight && styles.themeToggleLabelActive]}>Light</Text>
                  </Pressable>
                  <View style={styles.themeToggleDivider} />
                  <Pressable
                    style={[styles.themeToggleHalf, isDark && styles.themeToggleHalfActive]}
                    onPress={() => {
                      capture({
                        name: "ui.mobile_preference_toggled",
                        props: { name: "theme", value: "dark" },
                      });
                      setPreference("dark");
                    }}
                  >
                    <Svg width={16} height={16} viewBox="0 0 24 24" fill="none" stroke={darkColor} strokeWidth={2} strokeLinecap="round" strokeLinejoin="round">
                      <Path d="M21 12.79A9 9 0 1 1 11.21 3 7 7 0 0 0 21 12.79z" />
                    </Svg>
                    <Text style={[styles.themeToggleLabel, isDark && styles.themeToggleLabelActive]}>Dark</Text>
                  </Pressable>
                </>
              );
            })()}
          </View>
          <View style={styles.systemRow}>
            <Text style={styles.accountRowLabel}>Use system setting</Text>
            <Switch
              value={preference === "system"}
              onValueChange={(on) => {
                capture({
                  name: "ui.mobile_preference_toggled",
                  props: { name: "theme_system", value: on },
                });
                setPreference(on ? "system" : mode);
              }}
              trackColor={{ false: colors.borderSoft, true: colors.success }}
            />
          </View>
        </View>

        {/* Actions */}
        <View style={styles.actionsWrap}>
          <PrimaryButton
            label="Sign Out"
            kind="ghost"
            disabled={deleteAccountMutation.isPending || isOffline}
            onPress={handleSignOut}
          />
          <PrimaryButton
            label={deleteAccountMutation.isPending ? "Deleting..." : "Delete Account"}
            kind="danger"
            disabled={deleteAccountMutation.isPending || isOffline}
            onPress={handleDeleteAccount}
          />
        </View>

        {/* Legal links */}
        <View style={styles.legalRow}>
          <Pressable
            onPress={() => {
              capture({
                name: "ui.mobile_legal_page_opened",
                props: { page: "terms" },
              });
              navigation.navigate("TermsOfService");
            }}
          >
            <Text style={styles.legalLink}>Terms of Service</Text>
          </Pressable>
          <Text style={styles.legalDot}>·</Text>
          <Pressable
            onPress={() => {
              capture({
                name: "ui.mobile_legal_page_opened",
                props: { page: "privacy" },
              });
              navigation.navigate("PrivacyPolicy");
            }}
          >
            <Text style={styles.legalLink}>Privacy Policy</Text>
          </Pressable>
        </View>
      </ScrollView>
      <TelegramLinkModal
        visible={isTelegramLinkVisible}
        onDismiss={() => setIsTelegramLinkVisible(false)}
        onConnected={handleTelegramConnected}
      />
      <ToastOverlay toast={toast} />
    </ScreenContainer>
  );
}

const createStyles = (c: ThemeColors) => StyleSheet.create({
  identityHeader: {
    flexDirection: "row",
    alignItems: "center",
    gap: spacing.lg,
    marginBottom: spacing.xxl,
  },
  avatarCircle: {
    width: 44,
    height: 44,
    borderRadius: radius.full,
    backgroundColor: c.primary,
    alignItems: "center",
    justifyContent: "center",
  },
  avatarImage: {
    width: 44,
    height: 44,
    borderRadius: radius.full,
  },
  avatarText: {
    ...typeScale.h2,
    color: c.onPrimary,
  },
  identityInfo: {
    flex: 1,
    minWidth: 0,
  },
  identityName: {
    ...typeScale.h2,
    color: c.textPrimary,
  },
  identityEmail: {
    ...typeScale.caption,
    color: c.textMuted,
    marginTop: 1,
  },
  statusBadge: {
    paddingHorizontal: spacing.md,
    paddingVertical: 3,
    borderRadius: radius.pill,
    backgroundColor: c.successSoft,
    borderWidth: 1,
    borderColor: c.success,
  },
  statusBadgeOffline: {
    backgroundColor: c.dangerSoftBg,
    borderColor: c.danger,
  },
  statusBadgeText: {
    ...typeScale.overline,
    color: c.success,
  },
  statusBadgeTextOffline: {
    color: c.danger,
  },
  card: {
    borderRadius: radius.lg,
    borderWidth: 1,
    borderColor: c.borderSoft,
    backgroundColor: c.card,
    padding: spacing.xl,
    gap: 0,
    marginBottom: spacing.xl,
  },
  cardTitle: {
    ...typeScale.title,
    color: c.textPrimary,
    marginBottom: 0,
  },
  accountRow: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
    paddingVertical: spacing.xl,
    borderBottomWidth: 1,
    borderBottomColor: c.borderSoft,
  },
  accountRowLast: {
    borderBottomWidth: 0,
  },
  accountRowLabel: {
    ...typeScale.description,
    color: c.textSecondary,
  },
  accountRowRight: {
    flexDirection: "row",
    alignItems: "center",
    gap: spacing.xs,
  },
  accountRowValue: {
    ...typeScale.label,
    color: c.textPrimary,
  },
  accountRowArrow: {
    ...typeScale.description,
    color: c.textMuted,
  },
  channelRowLeft: {
    flexDirection: "row",
    alignItems: "center",
    gap: spacing.sm,
    flex: 1,
  },
  connectedPill: {
    flexDirection: "row",
    alignItems: "center",
    gap: spacing.xs,
    backgroundColor: c.primaryGlow,
    borderWidth: 1,
    borderColor: c.primary,
    borderRadius: radius.pill,
    paddingHorizontal: spacing.sm,
    paddingVertical: 2,
  },
  connectedPillText: {
    ...typeScale.overline,
    color: c.primary,
  },
  linkPill: {
    backgroundColor: c.primaryGlow,
    borderWidth: 1,
    borderColor: c.primary,
    borderRadius: radius.pill,
    paddingHorizontal: spacing.md,
    paddingVertical: 3,
  },
  linkPillText: {
    ...typeScale.overline,
    color: c.primary,
  },
  channelHint: {
    ...typeScale.small,
    color: c.textMuted,
    marginTop: spacing.xs,
  },
  themeToggleWrap: {
    flexDirection: "row",
    borderRadius: radius.sm,
    borderWidth: 1,
    borderColor: c.borderSoft,
    overflow: "hidden",
    marginTop: spacing.sm,
  },
  themeToggleHalf: {
    flex: 1,
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "center",
    gap: spacing.xs + spacing.xxs,
    paddingVertical: spacing.lg,
  },
  themeToggleHalfActive: {
    backgroundColor: c.primaryGlow,
  },
  themeToggleDivider: {
    width: 1,
    backgroundColor: c.borderSoft,
  },
  themeToggleLabel: {
    ...typeScale.label,
    color: c.textMuted,
  },
  themeToggleLabelActive: {
    color: c.primary,
  },
  systemRow: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
    marginTop: spacing.md,
  },
  actionsWrap: {
    gap: spacing.md,
    marginTop: spacing.xs,
  },
  metaText: {
    color: c.textSecondary,
    ...typeScale.body,
    paddingVertical: spacing.md,
  },
  errorText: {
    color: c.dangerSoft,
    ...typeScale.caption,
    paddingVertical: spacing.md,
  },
  offlineNote: {
    ...typeScale.caption,
    color: c.textMuted,
    textAlign: "center",
    paddingVertical: spacing.md,
  },
  legalRow: {
    flexDirection: "row",
    justifyContent: "center",
    alignItems: "center",
    gap: spacing.sm,
    marginTop: spacing.xxl,
    paddingBottom: spacing.md,
  },
  legalLink: {
    ...typeScale.caption,
    color: c.textMuted,
    textDecorationLine: "underline",
  },
  legalDot: {
    ...typeScale.caption,
    color: c.textMuted,
  },
});
