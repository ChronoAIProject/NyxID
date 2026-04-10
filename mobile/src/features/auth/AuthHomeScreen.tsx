import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import FontAwesome from "@expo/vector-icons/FontAwesome";
import * as WebBrowser from "expo-web-browser";
import { NativeStackScreenProps } from "@react-navigation/native-stack";
import { ActivityIndicator, Linking, Pressable, ScrollView, StyleSheet, Text, TextInput, View } from "react-native";
import type { RootStackParamList } from "../../app/AppNavigator";

import { ScreenContainer } from "../../components/ScreenContainer";
import { SectionBadge } from "../../components/SectionBadge";
import { ToastKind, ToastOverlay, ToastState } from "../../components/ToastOverlay";
import { mobileApi } from "../../lib/api/mobileApi";
import { useAuthSession } from "./AuthSessionContext";
import { IS_DEV_BUILD } from "../../lib/env";
import { useTheme } from "../../theme/ThemeContext";
import type { ThemeColors } from "../../theme/mobileTheme";
import { createFlowStyles } from "../../theme/flowStyles";
import { radius, spacing, typeScale } from "../../theme/designTokens";

type SocialProvider = "google" | "github" | "apple";
type Props = NativeStackScreenProps<RootStackParamList, "Auth">;
const SOCIAL_CALLBACK_URL = "nyxid://auth/social/callback";

type SocialCallback = {
  status: "success" | "error";
  accessToken?: string;
  refreshToken?: string;
  expiresIn?: number;
  error?: string;
  provider?: SocialProvider;
};

function resolveAuthError(error: unknown): string {
  if (!(error instanceof Error)) return "Sign-in failed. Please try again.";
  return error.message || "Sign-in failed. Please try again.";
}

function resolveSocialAuthError(error: string | undefined): string {
  switch (error) {
    case "social_auth_denied":
      return "Social sign-in was cancelled.";
    case "social_auth_csrf":
      return "Social sign-in failed security check. Please retry.";
    case "social_auth_conflict":
      return "This email is linked to another login method.";
    case "social_auth_no_email":
      return "Provider did not return a verified email.";
    case "social_auth_deactivated":
      return "This account is deactivated.";
    case "social_auth_exchange":
    case "social_auth_profile":
      return "Unable to complete social sign-in.";
    default:
      return "Social sign-in failed. Please try again.";
  }
}

function parseSocialCallback(url: string): SocialCallback | null {
  if (!url.startsWith(SOCIAL_CALLBACK_URL)) {
    return null;
  }

  try {
    const parsed = new URL(url);
    const statusRaw = parsed.searchParams.get("status");
    const providerRaw = parsed.searchParams.get("provider");
    const provider: SocialProvider | undefined =
      providerRaw === "google" || providerRaw === "github" || providerRaw === "apple"
        ? providerRaw
        : undefined;
    if (statusRaw !== "success" && statusRaw !== "error") {
      return null;
    }

    if (statusRaw === "error") {
      return {
        status: "error",
        error: parsed.searchParams.get("error") ?? undefined,
        provider,
      };
    }

    const expiresInRaw = parsed.searchParams.get("expires_in");
    const expiresInParsed = expiresInRaw ? Number(expiresInRaw) : NaN;

    return {
      status: "success",
      accessToken: parsed.searchParams.get("access_token") ?? undefined,
      refreshToken: parsed.searchParams.get("refresh_token") ?? undefined,
      expiresIn:
        Number.isFinite(expiresInParsed) && expiresInParsed > 0 ? expiresInParsed : undefined,
      provider,
    };
  } catch {
    return null;
  }
}

function SocialAuthButton({
  label,
  provider,
  disabled = false,
  loading = false,
  onPress,
}: {
  label: string;
  provider: SocialProvider;
  disabled?: boolean;
  loading?: boolean;
  onPress: () => void;
}) {
  const { colors } = useTheme();
  const styles = useMemo(() => createStyles(colors), [colors]);
  const iconName = provider === "google" ? "google" : provider === "github" ? "github" : "apple";
  const iconColor = colors.textPrimary;

  return (
    <Pressable onPress={onPress} disabled={disabled} style={[styles.socialAuthButton, disabled && !loading && styles.socialAuthButtonDisabled]}>
      <View style={styles.socialAuthContent}>
        {loading ? (
          <ActivityIndicator size="small" color={iconColor} />
        ) : (
          <FontAwesome name={iconName} size={16} color={iconColor} />
        )}
        <Text style={styles.socialAuthText}>{loading ? "Connecting..." : label}</Text>
      </View>
    </Pressable>
  );
}

export function AuthHomeScreen({ navigation }: Props) {
  const { colors } = useTheme();
  const styles = useMemo(() => createStyles(colors), [colors]);
  const flowStyles = useMemo(() => createFlowStyles(colors), [colors]);
  const [isSocialAuthPending, setIsSocialAuthPending] = useState(false);
  const [pendingSocialProvider, setPendingSocialProvider] = useState<SocialProvider | null>(null);
  const [toast, setToast] = useState<ToastState | null>(null);
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [isEmailAuthPending, setIsEmailAuthPending] = useState(false);
  const { signInWithSession } = useAuthSession();
  const isMountedRef = useRef(true);
  const lastHandledSocialUrlRef = useRef<string | null>(null);

  const showToast = (message: string, kind: ToastKind) => {
    setToast({ message, kind });
  };

  useEffect(() => {
    return () => {
      isMountedRef.current = false;
    };
  }, []);

  useEffect(() => {
    if (!toast) return;
    const timer = setTimeout(() => setToast(null), 2200);
    return () => clearTimeout(timer);
  }, [toast]);

  const handleSocialCallback = useCallback(
    async (url: string) => {
      if (lastHandledSocialUrlRef.current === url) {
        return;
      }

      const callback = parseSocialCallback(url);
      if (!callback) {
        return;
      }

      lastHandledSocialUrlRef.current = url;

      if (callback.status === "error") {
        showToast(resolveSocialAuthError(callback.error), "error");
        if (isMountedRef.current) {
          setIsSocialAuthPending(false);
          setPendingSocialProvider(null);
        }
        return;
      }

      if (!callback.accessToken) {
        showToast("Missing social auth access token.", "error");
        if (isMountedRef.current) {
          setIsSocialAuthPending(false);
          setPendingSocialProvider(null);
        }
        return;
      }

      if (isMountedRef.current) {
        setToast(null);
        setIsSocialAuthPending(true);
        setPendingSocialProvider((current) => callback.provider ?? current);
      }

      try {
        await signInWithSession({
          accessToken: callback.accessToken,
          refreshToken: callback.refreshToken,
          accessTokenExpiresAt:
            typeof callback.expiresIn === "number"
              ? Date.now() + Math.floor(callback.expiresIn * 1000)
              : undefined,
        });
      } catch (error) {
        showToast(resolveAuthError(error), "error");
      } finally {
        if (isMountedRef.current) {
          setIsSocialAuthPending(false);
          setPendingSocialProvider(null);
        }
      }
    },
    [signInWithSession]
  );

  useEffect(() => {
    void Linking.getInitialURL().then((url) => {
      if (!url) return;
      void handleSocialCallback(url);
    });

    const subscription = Linking.addEventListener("url", ({ url }) => {
      void handleSocialCallback(url);
    });

    return () => {
      subscription.remove();
    };
  }, [handleSocialCallback]);

  const startSocialLogin = async (provider: SocialProvider) => {
    if (isSocialAuthPending) {
      return;
    }

    if (isMountedRef.current) {
      setToast(null);
      setIsSocialAuthPending(true);
      setPendingSocialProvider(provider);
    }

    try {
      const authorizeUrl = mobileApi.getSocialAuthorizeUrl(provider, SOCIAL_CALLBACK_URL);
      const result = await WebBrowser.openAuthSessionAsync(
        authorizeUrl,
        SOCIAL_CALLBACK_URL
      );

      if (result.type === "success") {
        await handleSocialCallback(result.url);
        return;
      }

      if (result.type === "cancel" || result.type === "dismiss") {
        showToast("Social sign-in was cancelled.", "info");
        return;
      }

      showToast("Unable to complete social sign-in.", "error");
    } catch (error) {
      const message = error instanceof Error ? error.message : "Failed to start social sign-in.";
      showToast(message, "error");
    } finally {
      if (isMountedRef.current) {
        setIsSocialAuthPending(false);
        setPendingSocialProvider(null);
      }
    }
  };

  const handleEmailLogin = async () => {
    if (isEmailAuthPending || !email.trim() || !password) return;
    setIsEmailAuthPending(true);
    setToast(null);
    try {
      const result = await mobileApi.loginWithPassword({ email: email.trim(), password });
      await signInWithSession({
        accessToken: result.accessToken,
        refreshToken: result.refreshToken,
        accessTokenExpiresAt: Date.now() + Math.floor(result.expiresIn * 1000),
      });
    } catch (error) {
      showToast(resolveAuthError(error), "error");
    } finally {
      if (isMountedRef.current) {
        setIsEmailAuthPending(false);
      }
    }
  };

  return (
    <ScreenContainer>
      <ScrollView
        style={flowStyles.content}
        contentContainerStyle={[flowStyles.scrollContent, styles.scrollContentExtra, { paddingHorizontal: spacing.xxl }]}
        showsVerticalScrollIndicator={false}
      >
        <SectionBadge label={IS_DEV_BUILD ? "DEV MODE" : "SOCIAL ONLY"} tone="info" />
        <Text style={flowStyles.title}>Continue to NyxID</Text>
        <Text style={flowStyles.subtitle}>Use Google, GitHub, or Apple to continue.</Text>

        <View style={flowStyles.card}>
          <SocialAuthButton
            label="Continue with Google"
            provider="google"
            disabled={isSocialAuthPending}
            loading={isSocialAuthPending && pendingSocialProvider === "google"}
            onPress={() => void startSocialLogin("google")}
          />
          <SocialAuthButton
            label="Continue with GitHub"
            provider="github"
            disabled={isSocialAuthPending}
            loading={isSocialAuthPending && pendingSocialProvider === "github"}
            onPress={() => void startSocialLogin("github")}
          />
          <SocialAuthButton
            label="Continue with Apple"
            provider="apple"
            disabled={isSocialAuthPending}
            loading={isSocialAuthPending && pendingSocialProvider === "apple"}
            onPress={() => void startSocialLogin("apple")}
          />

          {IS_DEV_BUILD && (
            <>
              <View style={styles.dividerRow}>
                <View style={styles.dividerLine} />
                <Text style={styles.dividerText}>OR</Text>
                <View style={styles.dividerLine} />
              </View>
              <TextInput
                style={styles.devInput}
                placeholder="Email"
                placeholderTextColor={colors.textMuted}
                value={email}
                onChangeText={setEmail}
                keyboardType="email-address"
                autoCapitalize="none"
                autoComplete="email"
                editable={!isEmailAuthPending}
              />
              <TextInput
                style={styles.devInput}
                placeholder="Password"
                placeholderTextColor={colors.textMuted}
                value={password}
                onChangeText={setPassword}
                secureTextEntry
                editable={!isEmailAuthPending}
              />
              <Pressable
                onPress={() => void handleEmailLogin()}
                disabled={isEmailAuthPending || !email.trim() || !password}
                style={[styles.devSignInButton, (isEmailAuthPending || !email.trim() || !password) && styles.socialAuthButtonDisabled]}
              >
                <View style={styles.socialAuthContent}>
                  {isEmailAuthPending ? (
                    <ActivityIndicator size="small" color={colors.textPrimary} />
                  ) : null}
                  <Text style={styles.socialAuthText}>{isEmailAuthPending ? "Signing in..." : "Sign In"}</Text>
                </View>
              </Pressable>
            </>
          )}

          <Text style={styles.legal}>
            By continuing, you agree to{" "}
            <Text style={styles.legalLink} onPress={() => navigation.navigate("TermsOfService")}>
              Terms
            </Text>{" "}
            and{" "}
            <Text style={styles.legalLink} onPress={() => navigation.navigate("PrivacyPolicy")}>
              Privacy
            </Text>
            .
          </Text>
          <Text style={styles.legalNote}>
            Account deletion is permanent; signing in again with the same provider creates a new account.
          </Text>
        </View>
      </ScrollView>
      <ToastOverlay toast={toast} bottom={64} />
    </ScreenContainer>
  );
}

const createStyles = (c: ThemeColors) => StyleSheet.create({
  scrollContentExtra: {
    paddingBottom: spacing.xxxl,
  },
  legal: {
    color: c.textMuted,
    ...typeScale.caption,
    fontSize: 11,
    marginTop: spacing.sm,
  },
  legalNote: {
    color: c.textMuted,
    ...typeScale.caption,
    fontSize: 10,
    marginTop: spacing.xs,
  },
  legalLink: {
    color: c.textSecondary,
    ...typeScale.caption,
    fontSize: 11,
    textDecorationLine: "underline",
  },
  dividerRow: {
    flexDirection: "row",
    alignItems: "center",
    marginVertical: spacing.sm,
  },
  dividerLine: {
    flex: 1,
    height: 1,
    backgroundColor: c.border,
  },
  dividerText: {
    color: c.textMuted,
    ...typeScale.caption,
    fontSize: 11,
    marginHorizontal: spacing.sm,
  },
  devInput: {
    backgroundColor: c.cardSoft,
    borderColor: c.border,
    borderWidth: 1,
    borderRadius: radius.md,
    paddingVertical: spacing.md,
    paddingHorizontal: spacing.lg,
    color: c.textPrimary,
    ...typeScale.caption,
    fontSize: 13,
  },
  devSignInButton: {
    backgroundColor: c.primary,
    borderRadius: radius.md,
    paddingVertical: spacing.md,
    paddingHorizontal: spacing.lg,
    alignItems: "center",
    justifyContent: "center",
  },
  socialAuthButton: {
    backgroundColor: c.cardSoft,
    borderColor: c.border,
    borderWidth: 1,
    borderRadius: radius.md,
    paddingVertical: spacing.md,
    paddingHorizontal: spacing.lg,
    alignItems: "center",
    justifyContent: "center",
  },
  socialAuthButtonDisabled: {
    opacity: 0.5,
  },
  socialAuthContent: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "center",
    gap: spacing.sm,
  },
  socialAuthText: {
    color: c.textPrimary,
    ...typeScale.caption,
    fontWeight: "600",
    fontSize: 12,
  },
});
