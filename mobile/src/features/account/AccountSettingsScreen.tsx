import { useEffect, useMemo, useState } from "react";
import {
  Alert,
  Image,
  Linking,
  StyleSheet,
  View,
  useWindowDimensions,
  type TextProps,
} from "react-native";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  AudioLines,
  Bell,
  FileText,
  Lock,
  Mail,
  Palette,
  ScrollText,
  ShieldCheck,
  Wallet,
} from "lucide-react-native";
import type { WorkspaceScreenProps } from "../../app/navigationTypes";
import { DynamicTypeText as Text } from "../../components/DynamicTypeText";
import { OfflineBanner } from "../../components/OfflineBanner";
import {
  ToastOverlay,
  type ToastKind,
  type ToastState,
} from "../../components/ToastOverlay";
import {
  SettingsGroup,
  SettingsPage,
  SettingsRow,
  useSettingsStacked,
} from "../../components/settings/SettingsList";
import { useAuthSession } from "../auth/AuthSessionContext";
import { capture } from "../../lib/telemetry";
import { useNetworkStatus } from "../../hooks/useNetworkStatus";
import { mobileApi } from "../../lib/api/mobileApi";
import { assistantVoiceFeatures } from "../../lib/api/assistantVoiceApi";
import { getAuthEpoch } from "../../lib/auth/authEpoch";
import { AccountDeletionSheet } from "./AccountDeletionSheet";
import { isApiError } from "../../lib/api/ApiError";
import { useTheme } from "../../theme/ThemeContext";
import type { ThemeColors } from "../../theme/mobileTheme";
import { fonts, radius, spacing, typeScale } from "../../theme/designTokens";

type Props = WorkspaceScreenProps<"AccountSettings">;

/** Deep links name a Settings area; each one lives on its own sub-page. */
const SECTION_PAGES = {
  notifications: "NotificationSettings",
  display: "DisplaySettings",
  privacy: "PrivacySettings",
} as const;

export function AccountSettingsTitle({
  children,
  tintColor,
  allowFontScaling,
  onLayout,
}: Pick<TextProps, "allowFontScaling" | "onLayout"> & {
  children: string;
  tintColor?: string;
}) {
  return (
    <Text
      role="heading"
      aria-level={1}
      numberOfLines={1}
      allowFontScaling={allowFontScaling}
      onLayout={onLayout}
      style={{ color: tintColor, fontFamily: fonts.displaySemi, fontSize: 18 }}
    >
      {children}
    </Text>
  );
}

/** Published support contact (App Review guideline 1.2), as on the legal pages. */
const SUPPORT_EMAIL = "contact@chrono-ai.fun";

function resolveDeleteAccountError(error: unknown): {
  message: string;
  shouldForceSignOut: boolean;
} {
  // Use errorKey for reliable matching when available (machine-readable, stable)
  if (isApiError(error)) {
    const key = error.errorKey;
    if (
      key === "unauthorized" ||
      key === "authentication_failed" ||
      key === "token_expired"
    ) {
      return {
        message: "Session expired. Please sign in again.",
        shouldForceSignOut: true,
      };
    }
    if (key === "not_found" || error.statusCode === 404) {
      return {
        message: "Account not found or already deleted.",
        shouldForceSignOut: true,
      };
    }
  }

  const raw = error instanceof Error ? error.message : "";
  const lower = raw.toLowerCase();

  if (
    lower.includes("auth_session_missing") ||
    lower.includes("request_failed_401")
  ) {
    return {
      message: "Session expired. Please sign in again.",
      shouldForceSignOut: true,
    };
  }

  if (
    lower.includes("network request failed") ||
    lower.includes("failed to fetch")
  ) {
    return {
      message: "Network error. Check API server and try again.",
      shouldForceSignOut: false,
    };
  }

  const fallback =
    __DEV__ && raw ? raw : "Failed to delete account. Please try again.";
  return { message: fallback, shouldForceSignOut: false };
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

function getIdentityName(
  name?: string | null,
  provider?: string | null,
): string {
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

/**
 * Settings root: a menu of grouped rows (icon, label, chevron). Each row opens a
 * sub-page that holds the actual controls; only Sign Out and Delete Account act here.
 */
export function AccountSettingsScreen({ navigation, route }: Props) {
  const section = route?.params?.section;
  useEffect(() => {
    if (section) navigation.push(SECTION_PAGES[section]);
  }, [navigation, section]);
  const { colors } = useTheme();
  const stacked = useSettingsStacked();
  const { fontScale } = useWindowDimensions();
  const styles = useMemo(
    () => createStyles(colors, stacked, fontScale),
    [colors, stacked, fontScale],
  );
  const [toast, setToast] = useState<ToastState | null>(null);
  const queryClient = useQueryClient();
  const { signOut } = useAuthSession();
  const { isConnected, recheckConnection } = useNetworkStatus();

  const { data: profile, refetch: refetchProfile } = useQuery({
    queryKey: ["account", "profile"],
    queryFn: () => mobileApi.getAccountProfile(),
  });
  // Same key as the chat's voice gate, so Settings and chat agree.
  const voiceFeatures = useQuery({
    queryKey: ["workspace", "voice-features", getAuthEpoch()],
    queryFn: ({ signal }) => assistantVoiceFeatures(signal),
    enabled: isConnected,
    retry: false,
    staleTime: 30000,
  });
  const voiceEnabled = !!voiceFeatures.data?.includes("assistant:voice");

  const showToast = (message: string, kind: ToastKind) =>
    setToast({ message, kind });

  useEffect(() => {
    if (!toast) return;
    const timer = setTimeout(() => setToast(null), 2400);
    return () => clearTimeout(timer);
  }, [toast]);

  const deleteAccountMutation = useMutation({
    mutationFn: () => mobileApi.deleteAccount(),
    onSuccess: async (result) => {
      showToast(
        result.status === "complete"
          ? "Account erasure complete. You have been signed out."
          : "Account deletion accepted. Cleanup continues until verified. You have been signed out.",
        "success",
      );
      try {
        await signOut();
      } catch {}
      queryClient.clear();
    },
    onError: (error) => {
      const resolved = resolveDeleteAccountError(error);
      showToast(resolved.message, "error");
      if (resolved.shouldForceSignOut) {
        void signOut()
          .then(() => queryClient.clear())
          .catch(() => {});
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
          void signOut()
            .then(() => queryClient.clear())
            .catch(() => {});
        },
      },
    ]);
  };

  const [confirmingDelete, setConfirmingDelete] = useState(false);
  const handleDeleteAccount = () => {
    // Opening the confirmation sheet counts as opening the
    // delete_account_confirm dialog. Cancelling ends it on step 1 without an
    // abandonment event; the confirm path emits destructive_confirmed.
    capture({
      name: "ui.mobile_dialog_opened",
      props: {
        dialog_id: "delete_account_confirm",
        entry_point: "account_settings",
      },
    });
    setConfirmingDelete(true);
  };
  const confirmDeleteAccount = () => {
    setConfirmingDelete(false);
    capture({
      name: "ui.mobile_destructive_confirmed",
      props: { domain: "account", action: "delete_account" },
    });
    setToast(null);
    deleteAccountMutation.mutate();
  };

  const initials = getInitials(profile?.display_name, profile?.email);
  const identityName = getIdentityName(
    profile?.display_name,
    profile?.social_provider,
  );
  const isOffline = !isConnected;
  const deleting = deleteAccountMutation.isPending;

  return (
    <>
      <SettingsPage>
        {isOffline && (
          <OfflineBanner
            subtitle="Some features unavailable"
            onRetry={async () => {
              const online = await recheckConnection();
              if (online) void refetchProfile();
              else
                showToast("Still offline — will retry when connected", "error");
            }}
          />
        )}

        <SettingsGroup inset={false}>
          <View style={styles.identity}>
            <View style={styles.avatarCircle}>
              {profile?.avatar_url ? (
                <Image
                  source={{ uri: profile.avatar_url }}
                  style={styles.avatarImage}
                />
              ) : (
                <Text style={styles.avatarText}>{initials}</Text>
              )}
            </View>
            <View style={styles.identityInfo}>
              <Text style={styles.identityName}>{identityName}</Text>
              <Text style={styles.identityEmail}>
                {profile?.email ?? "..."}
              </Text>
              {profile ? (
                <Text style={styles.identityMethod}>
                  Signs in with {formatSignInMethod(profile.social_provider)}
                </Text>
              ) : null}
            </View>
            <View
              style={[
                styles.statusBadge,
                isOffline && styles.statusBadgeOffline,
              ]}
            >
              <Text
                style={[
                  styles.statusBadgeText,
                  isOffline && styles.statusBadgeTextOffline,
                ]}
              >
                {isOffline ? "Offline" : "Active"}
              </Text>
            </View>
          </View>
        </SettingsGroup>

        <SettingsGroup title="Account">
          <SettingsRow
            icon={ShieldCheck}
            label="Account & security"
            onPress={() => navigation.push("AccountSecurity")}
          />
          <SettingsRow
            icon={Wallet}
            label="Usage & credits"
            onPress={() => navigation.push("UsageCredits")}
          />
        </SettingsGroup>

        <SettingsGroup title="Preferences">
          <SettingsRow
            icon={Bell}
            label="Notifications"
            onPress={() => navigation.push("NotificationSettings")}
          />
          <SettingsRow
            icon={Palette}
            label="Display"
            onPress={() => navigation.push("DisplaySettings")}
          />
          {/* Dictation is available to everyone; live voice only with its flag. */}
          <SettingsRow
            icon={AudioLines}
            label={voiceEnabled ? "Voice" : "Dictation"}
            onPress={() =>
              navigation.push("VoiceSettings", { voice: voiceEnabled })
            }
          />
          <SettingsRow
            icon={Lock}
            label="Privacy"
            onPress={() => navigation.push("PrivacySettings")}
          />
        </SettingsGroup>

        <SettingsGroup title="Support">
          <SettingsRow
            icon={ScrollText}
            label="Terms of Service"
            onPress={() => {
              capture({
                name: "ui.mobile_legal_page_opened",
                props: { page: "terms" },
              });
              navigation.push("TermsOfService");
            }}
          />
          <SettingsRow
            icon={FileText}
            label="Privacy Policy"
            onPress={() => {
              capture({
                name: "ui.mobile_legal_page_opened",
                props: { page: "privacy" },
              });
              navigation.push("PrivacyPolicy");
            }}
          />
          <SettingsRow
            icon={Mail}
            label="Contact support"
            value={SUPPORT_EMAIL}
            accessibilityRole="link"
            accessibilityHint={`Email ${SUPPORT_EMAIL}`}
            onPress={() => {
              Linking.openURL(
                `mailto:${SUPPORT_EMAIL}?subject=${encodeURIComponent("NyxID app support")}`,
              ).catch(() =>
                Alert.alert("Contact support", `Email us at ${SUPPORT_EMAIL}.`),
              );
            }}
          />
        </SettingsGroup>

        <SettingsGroup>
          <SettingsRow
            centered
            label="Sign Out"
            disabled={deleting || isOffline}
            onPress={handleSignOut}
          />
        </SettingsGroup>

        <SettingsGroup
          footer="Permanently deletes your account and its data. This cannot be undone."
        >
          <SettingsRow
            centered
            destructive
            label={deleting ? "Deleting..." : "Delete Account"}
            disabled={deleting || isOffline}
            onPress={handleDeleteAccount}
          />
        </SettingsGroup>
      </SettingsPage>
      <AccountDeletionSheet
        visible={confirmingDelete}
        onCancel={() => setConfirmingDelete(false)}
        onConfirm={confirmDeleteAccount}
      />
      <ToastOverlay toast={toast} />
    </>
  );
}

const createStyles = (
  c: ThemeColors,
  stacked: boolean,
  fontScale: number,
  avatarSize = Math.ceil(44 * Math.max(1, fontScale)),
) =>
  StyleSheet.create({
    identity: {
      flexDirection: stacked ? "column" : "row",
      alignItems: stacked ? "flex-start" : "center",
      gap: spacing.lg,
      padding: spacing.xxl,
    },
    avatarCircle: {
      width: avatarSize,
      height: avatarSize,
      borderRadius: radius.full,
      backgroundColor: c.primary,
      alignItems: "center",
      justifyContent: "center",
    },
    avatarImage: {
      width: avatarSize,
      height: avatarSize,
      borderRadius: radius.full,
    },
    avatarText: {
      ...typeScale.h2,
      color: c.onPrimary,
    },
    identityInfo: {
      flex: stacked ? undefined : 1,
      width: stacked ? "100%" : undefined,
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
    // DESIGN.md §Badges: rounded-md (6px), px-2 py-0.5, text-[10px] font-medium.
    // Theme-aware via `successTone` / `dangerTone` so light + dark both render correctly.
    statusBadge: {
      maxWidth: "100%",
      flexShrink: 0,
      paddingHorizontal: spacing.sm,
      paddingVertical: 2,
      borderRadius: radius.sm,
      backgroundColor: c.successTone.bg,
      borderWidth: 1,
      borderColor: c.successTone.border,
    },
    statusBadgeOffline: {
      backgroundColor: c.dangerTone.bg,
      borderColor: c.dangerTone.border,
    },
    statusBadgeText: {
      ...typeScale.badge,
      color: c.successTone.text,
    },
    statusBadgeTextOffline: {
      color: c.dangerTone.text,
    },
    identityMethod: {
      ...typeScale.small,
      color: c.textMuted,
      marginTop: 1,
    },
  });
