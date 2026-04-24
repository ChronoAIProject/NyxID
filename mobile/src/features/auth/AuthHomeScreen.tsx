import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import FontAwesome from "@expo/vector-icons/FontAwesome";
import * as WebBrowser from "expo-web-browser";
import { NativeStackScreenProps } from "@react-navigation/native-stack";
import { ActivityIndicator, Linking, Pressable, ScrollView, StyleSheet, Text, TextInput, View } from "react-native";
import Svg, { Circle, Path, Defs, LinearGradient, Stop } from "react-native-svg";
import type { RootStackParamList } from "../../app/AppNavigator";

import { ScreenContainer } from "../../components/ScreenContainer";
import { ToastKind, ToastOverlay, ToastState } from "../../components/ToastOverlay";
import { mobileApi } from "../../lib/api/mobileApi";
import { resolveErrorMessage } from "../../lib/api/errorMessages";
import { useAuthSession } from "./AuthSessionContext";
import { capture } from "../../lib/telemetry";
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
    case "social_auth_registration_closed":
      return "WAITLIST";
    default:
      return error || "Social sign-in failed. Please try again.";
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
      return {
        status: "error",
        error: "social_auth_unknown",
        provider,
      };
    }

    if (statusRaw === "error") {
      return {
        status: "error",
        error: parsed.searchParams.get("error") ?? parsed.searchParams.get("message") ?? undefined,
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
  } catch (e) {
    return {
      status: "error",
      error: e instanceof Error ? e.message : "social_auth_unknown",
    };
  }
}

function PortalMarkLogo() {
  return (
    <View style={{ width: 96, height: 96, borderRadius: 48, backgroundColor: "#10101A", alignItems: "center", justifyContent: "center" }}>
    <Svg width={72} height={72} viewBox="0 0 130 130" fill="none">
      <Defs>
        <LinearGradient id="pl_o" gradientUnits="userSpaceOnUse" x1="10" y1="65" x2="120" y2="65">
          <Stop offset="0" stopColor="#A78BFA" />
          <Stop offset="0.5" stopColor="#A78BFA" stopOpacity={0} />
        </LinearGradient>
        <LinearGradient id="pl_m" gradientUnits="userSpaceOnUse" x1="10" y1="65" x2="120" y2="65" gradientTransform="rotate(120 65 65)">
          <Stop offset="0" stopColor="#C4B5FD" />
          <Stop offset="0.5" stopColor="#C4B5FD" stopOpacity={0} />
        </LinearGradient>
        <LinearGradient id="pl_i" gradientUnits="userSpaceOnUse" x1="10" y1="65" x2="120" y2="65" gradientTransform="rotate(240 65 65)">
          <Stop offset="0" stopColor="#DDD6FE" />
          <Stop offset="0.5" stopColor="#DDD6FE" stopOpacity={0} />
        </LinearGradient>
        <LinearGradient id="pl_v" gradientUnits="userSpaceOnUse" x1="56" y1="62" x2="86" y2="62" gradientTransform="rotate(160 71 62)">
          <Stop offset="0" stopColor="#C4B5FD" />
          <Stop offset="1" stopColor="#7C3AED" />
        </LinearGradient>
      </Defs>
      <Circle cx={65} cy={65} r={55} fill="none" stroke="url(#pl_o)" strokeWidth={1} />
      <Circle cx={65} cy={65} r={40} fill="none" stroke="url(#pl_m)" strokeWidth={1} />
      <Circle cx={65} cy={65} r={25} fill="none" stroke="url(#pl_i)" strokeWidth={0.8} />
      <Path d="M24 0q6 8 6 20 0 12-6 20-14-4-20-12-4-14-2-24 4-4 22-4z" transform="translate(56 42)" fill="url(#pl_v)" />
      <Circle cx={31.5} cy={49.5} r={1.5} fill="#C4B5FD" />
      <Circle cx={39} cy={63} r={1} fill="#C4B5FD" opacity={0.5} />
      <Circle cx={25} cy={69} r={1} fill="#C4B5FD" opacity={0.31} />
    </Svg>
    </View>
  );
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
  const [loginError, setLoginError] = useState<string | null>(null);
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

  const resetSocialState = useCallback(() => {
    if (isMountedRef.current) {
      setIsSocialAuthPending(false);
      setPendingSocialProvider(null);
    }
  }, []);

  const handleSocialCallback = useCallback(
    async (url: string) => {
      if (lastHandledSocialUrlRef.current === url) {
        return;
      }
      lastHandledSocialUrlRef.current = url;

      const callback = parseSocialCallback(url);
      if (__DEV__) console.log(`[auth] Parsed callback:`, JSON.stringify(callback));

      if (!callback) {
        setLoginError("Unable to complete social sign-in.");
        resetSocialState();
        return;
      }

      if (callback.status === "error") {
        if (__DEV__) console.log(`[auth] Social auth error:`, callback.error);
        setLoginError(resolveSocialAuthError(callback.error));
        resetSocialState();
        return;
      }

      if (!callback.accessToken) {
        setLoginError("Missing social auth access token.");
        resetSocialState();
        return;
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
        setLoginError(resolveErrorMessage(error));
      } finally {
        resetSocialState();
      }
    },
    [signInWithSession, resetSocialState]
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

    capture({
      name: "ui.mobile_provider_connect_initiated",
      props: { provider, method: "oauth" },
    });

    // Reset the dedup ref so a fresh attempt can re-process a callback URL
    // that matches a previous attempt's URL. The backend's mobile error
    // redirect is deterministic (status=error&error=<code>) so two retries
    // against the same invite-only gate produce identical URLs; without
    // this reset the second attempt is silently swallowed and the user
    // sees no error yet no login. The within-attempt race between
    // WebBrowser.openAuthSessionAsync and Linking's "url" listener is
    // still deduped because whichever path fires first sets the ref.
    lastHandledSocialUrlRef.current = null;

    if (isMountedRef.current) {
      setLoginError(null);
      setIsSocialAuthPending(true);
      setPendingSocialProvider(provider);
    }

    try {
      const authorizeUrl = mobileApi.getSocialAuthorizeUrl(provider, SOCIAL_CALLBACK_URL);
      if (__DEV__) console.log(`[auth] Opening ${provider} auth: ${authorizeUrl}`);

      const result = await WebBrowser.openAuthSessionAsync(
        authorizeUrl,
        SOCIAL_CALLBACK_URL
      );

      if (__DEV__) console.log(`[auth] Browser result:`, JSON.stringify(result));

      if (result.type === "success") {
        if (__DEV__) console.log(`[auth] Callback URL: ${result.url}`);
        await handleSocialCallback(result.url);
        return;
      }

      if (result.type === "cancel" || result.type === "dismiss") {
        setLoginError(`Sign-in was closed (${result.type}). Please try again.`);
        return;
      }

      setLoginError(`Social sign-in failed: ${result.type}`);
    } catch (error) {
      if (__DEV__) console.log(`[auth] startSocialLogin error:`, error);
      setLoginError(resolveErrorMessage(error));
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
    setLoginError(null);
    try {
      const result = await mobileApi.loginWithPassword({ email: email.trim(), password });
      await signInWithSession({
        accessToken: result.accessToken,
        refreshToken: result.refreshToken,
        accessTokenExpiresAt: Date.now() + Math.floor(result.expiresIn * 1000),
      });
    } catch (error) {
      setLoginError(resolveErrorMessage(error));
    } finally {
      if (isMountedRef.current) {
        setIsEmailAuthPending(false);
      }
    }
  };

  const isAnyPending = isSocialAuthPending || isEmailAuthPending;

  return (
    <ScreenContainer>
      <ScrollView
        style={flowStyles.content}
        contentContainerStyle={[flowStyles.scrollContent, styles.scrollContentExtra, { paddingHorizontal: spacing.xxl }]}
        showsVerticalScrollIndicator={false}
      >
        {/* Hero branding */}
        <View style={styles.heroWrap}>
          <PortalMarkLogo />
          <Text style={styles.heroTitle}>NyxID</Text>
          <Text style={styles.heroTagline}>Your companion for approvals and notifications</Text>
        </View>

        {/* Email login */}
        <View style={flowStyles.card}>
          <TextInput
            style={styles.input}
            placeholder="Email"
            placeholderTextColor={colors.textMuted}
            value={email}
            onChangeText={(v) => { setEmail(v); setLoginError(null); }}
            keyboardType="email-address"
            autoCapitalize="none"
            autoComplete="email"
          />
          <TextInput
            style={styles.input}
            placeholder="Password"
            placeholderTextColor={colors.textMuted}
            value={password}
            onChangeText={(v) => { setPassword(v); setLoginError(null); }}
            secureTextEntry
            autoComplete="current-password"
          />
          <Pressable
            onPress={() => void handleEmailLogin()}
            disabled={isEmailAuthPending || !email.trim() || !password}
            style={[styles.signInButton, (isEmailAuthPending || !email.trim() || !password) && styles.buttonDisabled]}
          >
            <View style={styles.socialAuthContent}>
              {isEmailAuthPending ? (
                <ActivityIndicator size="small" color={colors.onPrimary} />
              ) : null}
              <Text style={styles.signInButtonText}>{isEmailAuthPending ? "Signing in..." : "Sign In"}</Text>
            </View>
          </Pressable>

          {/* Divider */}
          <View style={styles.dividerRow}>
            <View style={styles.dividerLine} />
            <Text style={styles.dividerText}>or</Text>
            <View style={styles.dividerLine} />
          </View>

          {/* Social login */}
          <SocialAuthButton
            label="Continue with Google"
            provider="google"
            disabled={isAnyPending}
            loading={isSocialAuthPending && pendingSocialProvider === "google"}
            onPress={() => void startSocialLogin("google")}
          />
          <SocialAuthButton
            label="Continue with GitHub"
            provider="github"
            disabled={isAnyPending}
            loading={isSocialAuthPending && pendingSocialProvider === "github"}
            onPress={() => void startSocialLogin("github")}
          />
          <SocialAuthButton
            label="Continue with Apple"
            provider="apple"
            disabled={isAnyPending}
            loading={isSocialAuthPending && pendingSocialProvider === "apple"}
            onPress={() => void startSocialLogin("apple")}
          />

          {/* Legal */}
          <Text style={styles.legal}>
            By continuing, you agree to{" "}
            <Text
              style={styles.legalLink}
              onPress={() => {
                capture({
                  name: "ui.mobile_legal_page_opened",
                  props: { page: "terms" },
                });
                navigation.navigate("TermsOfService");
              }}
            >
              Terms
            </Text>{" "}
            and{" "}
            <Text
              style={styles.legalLink}
              onPress={() => {
                capture({
                  name: "ui.mobile_legal_page_opened",
                  props: { page: "privacy" },
                });
                navigation.navigate("PrivacyPolicy");
              }}
            >
              Privacy
            </Text>
            .
          </Text>
        </View>

        {loginError && (
          <View style={styles.errorBanner}>
            {loginError === "WAITLIST" ? (
              <>
                <Text style={styles.errorText}>
                  Registration is invite-only.{" "}
                  <Text
                    style={styles.errorLink}
                    onPress={() => void Linking.openURL("https://nyx.chrono-ai.fun/#waitlist")}
                  >
                    Join the waitlist
                  </Text>
                  {" "}to get access.
                </Text>
              </>
            ) : (
              <Text style={styles.errorText}>{loginError}</Text>
            )}
          </View>
        )}
      </ScrollView>
      <ToastOverlay toast={toast} bottom={64} />
    </ScreenContainer>
  );
}

const createStyles = (c: ThemeColors) => StyleSheet.create({
  scrollContentExtra: {
    paddingBottom: spacing.xxxl,
  },
  heroWrap: {
    alignItems: "center",
    gap: spacing.sm,
    marginBottom: spacing.sm,
    paddingTop: spacing.huge,
  },
  heroTitle: {
    ...typeScale.h1,
    color: c.textPrimary,
    marginTop: spacing.sm,
  },
  heroTagline: {
    ...typeScale.caption,
    color: c.textMuted,
    textAlign: "center",
  },
  input: {
    backgroundColor: c.cardSoft,
    borderColor: c.border,
    borderWidth: 1,
    borderRadius: radius.md,
    paddingVertical: spacing.md,
    paddingHorizontal: spacing.lg,
    color: c.textPrimary,
    ...typeScale.body,
    fontSize: 14,
  },
  signInButton: {
    backgroundColor: c.primary,
    borderRadius: radius.md,
    paddingVertical: spacing.lg,
    alignItems: "center",
    justifyContent: "center",
  },
  signInButtonText: {
    color: c.onPrimary,
    ...typeScale.bodyStrong,
    fontSize: 14,
  },
  buttonDisabled: {
    opacity: 0.5,
  },
  errorBanner: {
    backgroundColor: c.dangerSoftBg,
    borderWidth: 1,
    borderColor: c.riskHigh.border,
    borderRadius: radius.sm,
    paddingVertical: spacing.sm,
    paddingHorizontal: spacing.lg,
  },
  errorText: {
    color: c.danger,
    ...typeScale.caption,
    lineHeight: 18,
  },
  errorLink: {
    color: c.primary,
    textDecorationLine: "underline" as const,
  },
  dividerRow: {
    flexDirection: "row",
    alignItems: "center",
    marginVertical: spacing.xs,
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
  legal: {
    color: c.textMuted,
    ...typeScale.caption,
    fontSize: 11,
    marginTop: spacing.sm,
    textAlign: "center",
  },
  legalLink: {
    color: c.textSecondary,
    ...typeScale.caption,
    fontSize: 11,
    textDecorationLine: "underline",
  },
});
