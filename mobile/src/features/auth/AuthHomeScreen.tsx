import { mfaPrompt } from "./mfaPrompt";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import * as WebBrowser from "expo-web-browser";
import { useIsFocused } from "@react-navigation/native";
import { useQuery } from "@tanstack/react-query";
import { NativeStackScreenProps } from "@react-navigation/native-stack";
import {
  ActivityIndicator,
  AppState,
  Linking,
  Pressable,
  ScrollView,
  StyleSheet,
  Text,
  TextInput,
  View,
} from "react-native";
import Svg, { Path, Defs, LinearGradient, Stop } from "react-native-svg";
import { LinearGradient as ExpoLinearGradient } from "expo-linear-gradient";
import type { RootStackParamList } from "../../app/AppNavigator";

import { ScreenContainer } from "../../components/ScreenContainer";
import { ToastKind, ToastOverlay, ToastState } from "../../components/ToastOverlay";
import { runAuthAttempt } from "../../lib/auth/authAttempt";
import { MfaChallengeError } from "../../lib/auth/mfaChallenge";
import { ApiError } from "../../lib/api/ApiError";
import { authSessionGeneration } from "../../lib/auth/sessionStore";
import { mobileApi } from "../../lib/api/mobileApi";
import { NYXID_FRONTEND_URL } from "../../lib/env";
import { resolveErrorMessage } from "../../lib/api/errorMessages";
import { useAuthSession } from "./AuthSessionContext";
import { getPublicAuthConfig } from "../../lib/api/publicAuthApi";
import { requireSocialProvider } from "./publicAuthConfig";
import {
  parseSocialCallback,
  SOCIAL_CALLBACK_URL,
  type SocialProvider,
} from "./socialCallback";
import { capture } from "../../lib/telemetry";
import { NYXID_FRONTEND_URL } from "../../lib/env";
import { useTheme } from "../../theme/ThemeContext";
import type { ThemeColors } from "../../theme/mobileTheme";
import { createFlowStyles } from "../../theme/flowStyles";
import { radius, spacing, typeScale } from "../../theme/designTokens";

type Props = NativeStackScreenProps<RootStackParamList, "Auth">;

function resolveAuthError(error: unknown): string {
  if (!(error instanceof Error)) return "Sign-in failed. Please try again.";
  return error.message || "Sign-in failed. Please try again.";
}

function resolveSocialAuthError(error: string | undefined): string {
  switch (error) {
    case "social_auth_invalid_mfa":
      return "Your verification challenge is invalid. Please sign in again.";
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

// Inline render of mobile/assets/sources/app_icon.svg
// (kept in sync by hand — if the source changes, regenerate the PNG icons
// AND update this JSX so the login screen matches the launcher icon)
function PortalMarkLogo({ size = 96 }: { size?: number }) {
  return (
    <Svg width={size} height={size} viewBox="0 0 702 702" fill="none">
      <Defs>
        <LinearGradient
          id="ai_bg"
          x1="351"
          y1="0"
          x2="351"
          y2="702"
          gradientUnits="userSpaceOnUse"
        >
          <Stop offset="0" stopColor="#221250" />
          <Stop offset="1" stopColor="#070707" />
        </LinearGradient>
        <LinearGradient
          id="ai_n"
          x1="351"
          y1="140.062"
          x2="351"
          y2="561.933"
          gradientUnits="userSpaceOnUse"
        >
          <Stop offset="0" stopColor="#A672FB" />
          <Stop offset="1" stopColor="#5E00F5" />
        </LinearGradient>
        <LinearGradient
          id="ai_hl"
          x1="351"
          y1="140.062"
          x2="351"
          y2="561.933"
          gradientUnits="userSpaceOnUse"
        >
          <Stop offset="0" stopColor="#FFFFFF" stopOpacity={0.5} />
          <Stop offset="1" stopColor="#FFFFFF" stopOpacity={0} />
        </LinearGradient>
      </Defs>
      <Path
        fillRule="evenodd"
        clipRule="evenodd"
        d="M702 218.631C702 210.298 702.003 201.963 701.952 193.629C701.91 186.608 701.83 179.589 701.639 172.571C701.226 157.276 700.324 141.85 697.604 126.726C694.845 111.383 690.34 97.104 683.241 83.1637C676.262 69.462 667.146 56.924 656.268 46.0541C645.393 35.1842 632.852 26.0736 619.145 19.0988C605.19 11.9979 590.896 7.4945 575.536 4.73678C560.41 2.0208 544.98 1.1203 529.685 0.707109C522.662 0.517127 515.64 0.437045 508.616 0.393596C500.277 0.342479 491.938 0.346739 483.599 0.346739L386.779 0H314.364L219.257 0.346739C210.902 0.346739 202.547 0.342479 194.192 0.393596C187.153 0.437045 180.118 0.517127 173.082 0.707109C157.751 1.1203 142.286 2.02165 127.124 4.74104C111.744 7.49791 97.4282 11.9996 83.4547 19.0954C69.719 26.071 57.1504 35.1825 46.2524 46.0541C35.3562 56.9231 26.2226 69.4586 19.2307 83.1577C12.112 97.1048 7.59841 111.393 4.83303 126.744C2.11024 141.862 1.20804 157.283 0.793152 172.571C0.604022 179.59 0.522236 186.609 0.47964 193.629C0.428524 201.964 0 212.318 0 220.652L0.00255581 314.442L0 387.632L0.432783 483.415C0.432783 491.76 0.429375 500.106 0.47964 508.451C0.522236 515.482 0.604022 522.51 0.794004 529.538C1.20804 544.852 2.11195 560.3 4.83729 575.445C7.60182 590.808 12.1145 605.108 19.2273 619.066C26.22 632.788 35.3553 645.342 46.2524 656.227C57.1495 667.112 69.7147 676.235 83.4479 683.22C97.4299 690.33 111.753 694.839 127.142 697.601C142.297 700.321 157.757 701.223 173.082 701.636C180.118 701.826 187.154 701.907 194.193 701.95C202.548 702.001 210.902 701.997 219.257 701.997L315.224 702H387.818L483.599 701.997C491.938 701.997 500.277 702.001 508.616 701.95C515.64 701.907 522.662 701.826 529.685 701.636C544.986 701.222 560.421 700.319 575.554 697.597C590.904 694.836 605.192 690.328 619.139 683.222C632.848 676.238 645.392 667.114 656.268 656.227C667.144 645.344 676.26 632.791 683.239 619.072C690.342 605.107 694.847 590.801 697.607 575.427C700.325 560.289 701.226 544.846 701.64 529.538C701.83 522.509 701.91 515.481 701.952 508.451C702.004 500.106 702 491.76 702 483.415C702 483.415 701.995 389.323 701.995 387.632V314.365C701.995 313.116 702 218.631 702 218.631"
        fill="url(#ai_bg)"
      />
      <Path
        d="M561.938 227.109V474.887C561.938 522.96 522.965 561.933 474.891 561.933H353.39C352.071 561.933 351 560.862 351 559.543V330.962C351 328.524 347.783 327.649 346.55 329.752L211.071 560.752C210.641 561.483 209.857 561.933 209.011 561.933H142.453C141.133 561.933 140.062 560.862 140.062 559.543V142.453C140.062 141.133 141.133 140.062 142.453 140.062H278.299C279.618 140.062 280.689 141.133 280.689 142.453V371.034C280.689 373.471 283.906 374.346 285.139 372.243L420.623 141.243C421.053 140.512 421.837 140.062 422.683 140.062H474.887C522.96 140.062 561.933 179.035 561.933 227.109H561.938Z"
        fill="url(#ai_n)"
      />
      <Path
        d="M142.452 141.312H278.299C278.928 141.312 279.439 141.823 279.439 142.452V371.033C279.439 374.744 284.339 376.079 286.218 372.875L421.7 141.875C421.907 141.525 422.282 141.313 422.683 141.312H474.887C522.27 141.313 560.682 179.725 560.683 227.108V228.358H560.688V474.887C560.687 522.27 522.275 560.682 474.892 560.683H353.39C352.761 560.682 352.25 560.172 352.25 559.543V330.962C352.25 327.251 347.35 325.916 345.472 329.12L209.994 560.118C209.788 560.469 209.412 560.683 209.011 560.683H142.452C141.823 560.682 141.313 560.172 141.312 559.543V142.452C141.313 141.824 141.824 141.313 142.452 141.312Z"
        stroke="url(#ai_hl)"
        strokeOpacity={0.5}
        strokeWidth={2.5}
        fill="none"
      />
    </Svg>
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
  const iconColor = colors.textPrimary;

  return (
    <Pressable
      onPress={onPress}
      disabled={disabled}
      accessibilityRole="button"
      accessibilityLabel={label}
      accessibilityState={{ disabled, busy: loading }}
      style={({ pressed }) => [
        styles.socialAuthButton,
        pressed && styles.socialAuthButtonPressed,
        disabled && styles.socialAuthButtonDisabled,
      ]}
    >
      <View style={styles.providerIcon} accessibilityElementsHidden importantForAccessibility="no-hide-descendants">
        {loading ? (
          <ActivityIndicator size="small" color={iconColor} />
        ) : (
          <SocialProviderIcon provider={provider} color={iconColor} />
        )}
      </View>
      <Text style={styles.providerLabel}>
        {loading ? "Connecting..." : label}
      </Text>
      <Svg width={16} height={16} viewBox="0 0 24 24" fill="none" accessible={false}>
        <Path d="m9 18 6-6-6-6" stroke={colors.textMuted} strokeWidth={2} strokeLinecap="round" strokeLinejoin="round" />
      </Svg>
    </Pressable>
  );
}

// Same provider marks as the web sign-in rows.
function SocialProviderIcon({ provider, color }: { provider: SocialProvider; color: string }) {
  return (
    <Svg width={16} height={16} viewBox="0 0 24 24" fill={color} accessible={false}>
      {provider === "google" ? <>
        <Path d="M22.56 12.25c0-.78-.07-1.53-.2-2.25H12v4.26h5.92a5.06 5.06 0 01-2.2 3.32v2.77h3.57c2.08-1.92 3.28-4.74 3.28-8.1z" fill="#4285F4" />
        <Path d="M12 23c2.97 0 5.46-.98 7.28-2.66l-3.57-2.77c-.98.66-2.23 1.06-3.71 1.06-2.86 0-5.29-1.93-6.16-4.53H2.18v2.84C3.99 20.53 7.7 23 12 23z" fill="#34A853" />
        <Path d="M5.84 14.09c-.22-.66-.35-1.36-.35-2.09s.13-1.43.35-2.09V7.07H2.18C1.43 8.55 1 10.22 1 12s.43 3.45 1.18 4.93l2.85-2.22.81-.62z" fill="#FBBC05" />
        <Path d="M12 5.38c1.62 0 3.06.56 4.21 1.64l3.15-3.15C17.45 2.09 14.97 1 12 1 7.7 1 3.99 3.47 2.18 7.07l3.66 2.84c.87-2.6 3.3-4.53 6.16-4.53z" fill="#EA4335" />
      </> : provider === "github" ? (
        <Path d="M12 0c-6.626 0-12 5.373-12 12 0 5.302 3.438 9.8 8.207 11.387.599.111.793-.261.793-.577v-2.234c-3.338.726-4.033-1.416-4.033-1.416-.546-1.387-1.333-1.756-1.333-1.756-1.089-.745.083-.729.083-.729 1.205.084 1.839 1.237 1.839 1.237 1.07 1.834 2.807 1.304 3.492.997.107-.775.418-1.305.762-1.604-2.665-.305-5.467-1.334-5.467-5.931 0-1.311.469-2.381 1.236-3.221-.124-.303-.535-1.524.117-3.176 0 0 1.008-.322 3.301 1.23.957-.266 1.983-.399 3.003-.404 1.02.005 2.047.138 3.006.404 2.291-1.552 3.297-1.23 3.297-1.23.653 1.653.242 2.874.118 3.176.77.84 1.235 1.911 1.235 3.221 0 4.609-2.807 5.624-5.479 5.921.43.372.823 1.102.823 2.222v3.293c0 .319.192.694.801.576 4.765-1.589 8.199-6.086 8.199-11.386 0-6.627-5.373-12-12-12z" />
      ) : (
        <Path d="M17.05 20.28c-.98.95-2.05.88-3.08.4-1.09-.5-2.08-.48-3.24 0-1.44.62-2.2.44-3.06-.4C2.79 15.25 3.51 7.59 9.05 7.31c1.35.07 2.29.74 3.08.8 1.18-.24 2.31-.93 3.57-.84 1.51.12 2.65.72 3.4 1.8-3.12 1.87-2.38 5.98.48 7.13-.57 1.5-1.31 2.99-2.54 4.09zM12.03 7.25c-.15-2.23 1.66-4.07 3.74-4.25.29 2.58-2.34 4.5-3.74 4.25z" />
      )}
    </Svg>
  );
}

export function AuthHomeScreen({ navigation }: Props) {
  const focused = useIsFocused();
  const focusedRef = useRef(focused);
  focusedRef.current = focused;
  const [foreground, setForeground] = useState(
    AppState.currentState === "active",
  );
  const publicConfig = useQuery({
    queryKey: ["public-auth-config"],
    queryFn: ({ signal }) => getPublicAuthConfig(signal),
    enabled: focused && foreground,
    staleTime: 30_000,
    refetchInterval: focused && foreground ? 30_000 : false,
    retry: false,
    networkMode: "always",
  });
  const loginConfig = publicConfig.isError ? undefined : publicConfig.data;
  const { colors } = useTheme();
  const styles = useMemo(() => createStyles(colors), [colors]);
  const flowStyles = useMemo(() => createFlowStyles(colors), [colors]);
  const [isSocialAuthPending, setIsSocialAuthPending] = useState(false);
  const [pendingSocialProvider, setPendingSocialProvider] =
    useState<SocialProvider | null>(null);
  const [toast, setToast] = useState<ToastState | null>(null);
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [mfaProvider, setMfaProvider] = useState<string | null>(null);
  const [recoveryMode, setRecoveryMode] = useState(false);
  const [mfaChallenge, setMfaChallenge] = useState<MfaChallengeError | null>(null);
  const [mfaCode, setMfaCode] = useState("");
  const [isEmailAuthPending, setIsEmailAuthPending] = useState(false);
  const socialLoginLock = useRef(false);
  const [loginError, setLoginError] = useState<string | null>(null);
  const { signInWithSession } = useAuthSession();
  const isMountedRef = useRef(true);
  const emailLoginLock = useRef(false);
  const lastHandledSocialUrlRef = useRef<string | null>(null);

  const showToast = (message: string, kind: ToastKind) => {
    setToast({ message, kind });
  };

  useEffect(() => {
    isMountedRef.current = true;
    return () => {
      isMountedRef.current = false;
    };
  }, []);

  useEffect(() => {
    if (!toast) return;
    const timer = setTimeout(() => setToast(null), 2200);
    return () => clearTimeout(timer);
  }, [toast]);

  useEffect(() => {
    const subscription = AppState.addEventListener("change", (state) => {
      setForeground(state === "active");
      if (state === "background") {
        setPassword("");
        setMfaCode("");
      }
    });
    return () => subscription.remove();
  }, []);

  const resetSocialState = useCallback(() => {
    if (isMountedRef.current) {
      setIsSocialAuthPending(false);
      setPendingSocialProvider(null);
    }
  }, []);

  const handleSocialCallback = useCallback(
    async (url: string) => {
      const callback = parseSocialCallback(url);
      if (!callback) return;
      if (lastHandledSocialUrlRef.current === url) {
        return;
      }
      lastHandledSocialUrlRef.current = url;

      if (callback.status === "error") {
        setLoginError(resolveSocialAuthError(callback.error));
        resetSocialState();
        return;
      }

      if (callback.status === "mfa_required" && callback.mfaToken) {
        setMfaProvider(callback.provider ?? "social");
        setRecoveryMode(false);
        setMfaChallenge(new MfaChallengeError(callback.mfaToken));
        setMfaCode("");
        setLoginError(null);
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
    [signInWithSession, resetSocialState],
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
    if (socialLoginLock.current || emailLoginLock.current) {
      return;
    }
    socialLoginLock.current = true;

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
      const available = await publicConfig.refetch();
      if (!isMountedRef.current || !focusedRef.current || AppState.currentState !== "active") return;
      if (available.error) throw available.error;
      requireSocialProvider(available.data, provider);
      capture({ name: "ui.mobile_provider_connect_initiated", props: { provider, method: "oauth" } });
      const authorizeUrl = mobileApi.getSocialAuthorizeUrl(provider, SOCIAL_CALLBACK_URL);

      const result = await WebBrowser.openAuthSessionAsync(
        authorizeUrl,
        SOCIAL_CALLBACK_URL,
      );


      if (result.type === "success") {
        await handleSocialCallback(result.url);
        return;
      }

      if (result.type === "cancel" || result.type === "dismiss") {
        setLoginError(`Sign-in was closed (${result.type}). Please try again.`);
        return;
      }

      setLoginError(`Social sign-in failed: ${result.type}`);
    } catch (error) {
      setLoginError(resolveErrorMessage(error));
    } finally {
      socialLoginLock.current = false;
      if (isMountedRef.current) {
        setIsSocialAuthPending(false);
        setPendingSocialProvider(null);
      }
    }
  };

  const handleEmailLogin = async () => {
    if (emailLoginLock.current || socialLoginLock.current || (!mfaChallenge && (!email.trim() || !password))) return;
    emailLoginLock.current = true;
    const generation = authSessionGeneration();
    setIsEmailAuthPending(true);
    setLoginError(null);
    const current = () => isMountedRef.current && authSessionGeneration() === generation;
    try {
      await runAuthAttempt(
        () => mfaChallenge
          ? mfaChallenge.verify(mfaCode, mobileApi.verifyMfa)
          : mobileApi.loginWithPassword({ email: email.trim(), password }),
        current,
        async (result) => {
          setMfaProvider(null); setRecoveryMode(false); setMfaChallenge(null); setMfaCode(""); setPassword("");
          await signInWithSession({
            accessToken: result.accessToken,
            refreshToken: result.refreshToken,
            accessTokenExpiresAt: Date.now() + Math.floor(result.expiresIn * 1000),
          });
        },
        (error) => {
          if (error instanceof MfaChallengeError) {
            setMfaProvider(null); setRecoveryMode(false); setMfaChallenge(error); setPassword(""); setMfaCode("");
          } else {
            setMfaCode("");
            if (error instanceof ApiError && error.reason === "mfa_challenge_expired") {
              setMfaProvider(null); setRecoveryMode(false); setMfaChallenge(null);
              setLoginError("Your verification expired. Please sign in again.");
            } else { setLoginError(resolveErrorMessage(error)); }
          }
        },
      );
    } finally {
      emailLoginLock.current = false;
      if (isMountedRef.current) {
        setIsEmailAuthPending(false);
      }
    }
  };

  const prompt = mfaPrompt(mfaProvider, recoveryMode);
  const isAnyPending = isSocialAuthPending || isEmailAuthPending;

  const openRegistration = async () => {
    if (!loginConfig?.emailAuthEnabled || isAnyPending || mfaChallenge) return;
    try {
      await WebBrowser.openBrowserAsync(
        new URL("/register", NYXID_FRONTEND_URL).href,
      );
    } catch (error) {
      setLoginError(resolveAuthError(error));
    }
  };

  return (
    <ScreenContainer>
      <ScrollView
        style={flowStyles.content}
        contentContainerStyle={[
          flowStyles.scrollContent,
          styles.scrollContentExtra,
          { paddingHorizontal: spacing.xxl },
        ]}
        showsVerticalScrollIndicator={false}
        keyboardShouldPersistTaps="handled"
      >
        {/* Hero branding */}
        <View style={styles.heroWrap}>
          <PortalMarkLogo />
          <Text style={styles.heroTitle}>NyxID</Text>
          <Text style={styles.heroTagline}>
            Your conversations, connections and approvals
          </Text>
        </View>

        {/* Social providers follow server config; existing accounts can always sign in. */}
        <View style={flowStyles.card}>
          {/* Social login */}
          {publicConfig.isPending ? (
            <View style={styles.socialAuthContent}>
              <ActivityIndicator size="small" color={colors.primary} />
              <Text
                style={{ ...typeScale.caption, color: colors.textSecondary }}
              >
                Loading sign-in options...
              </Text>
            </View>
          ) : publicConfig.isError ? (
            <View style={{ gap: spacing.md }}>
              <Text
                style={{ ...typeScale.caption, color: colors.textSecondary }}
              >
                Could not load sign-in options. Please try again.
              </Text>
              <Pressable
                accessibilityRole="button"
                disabled={publicConfig.isFetching}
                onPress={() => void publicConfig.refetch()}
                style={styles.socialAuthButton}
              >
                <Text style={styles.socialAuthText}>
                  {publicConfig.isFetching
                    ? "Retrying..."
                    : "Retry sign-in options"}
                </Text>
              </Pressable>
            </View>
          ) : null}
          {loginConfig?.socialProviders.includes("google") ? (
            <SocialAuthButton
              label="Continue with Google"
              provider="google"
              disabled={isAnyPending}
              loading={
                isSocialAuthPending && pendingSocialProvider === "google"
              }
              onPress={() => void startSocialLogin("google")}
            />
          ) : null}
          {loginConfig?.socialProviders.includes("github") ? (
            <SocialAuthButton
              label="Continue with GitHub"
              provider="github"
              disabled={isAnyPending}
              loading={
                isSocialAuthPending && pendingSocialProvider === "github"
              }
              onPress={() => void startSocialLogin("github")}
            />
          ) : null}
          {loginConfig?.socialProviders.includes("apple") ? (
            <SocialAuthButton
              label="Continue with Apple"
              provider="apple"
              disabled={isAnyPending}
              loading={isSocialAuthPending && pendingSocialProvider === "apple"}
              onPress={() => void startSocialLogin("apple")}
            />
          ) : null}

            <>
              {loginConfig && loginConfig.socialProviders.length > 0 ? (
                <View style={styles.dividerRow}>
                  <View style={styles.dividerLine} />
                  <Text style={styles.dividerText}>or</Text>
                  <View style={styles.dividerLine} />
                </View>
              ) : null}
              {prompt.showPasswordFields && (
                <>
                  <TextInput
                    style={styles.input}
                    accessibilityLabel="Email"
                    placeholder="Email"
                    placeholderTextColor={colors.textMuted}
                    value={email}
                    onChangeText={(v) => { setEmail(v); setMfaChallenge(null); setMfaCode(""); setLoginError(null); }}
                    editable={!isAnyPending && !mfaChallenge}
                    keyboardType="email-address"
                    autoCapitalize="none"
                    autoComplete="email"
                  />
                  <TextInput
                    style={styles.input}
                    accessibilityLabel="Password"
                    placeholder="Password"
                    placeholderTextColor={colors.textMuted}
                    value={password}
                    onChangeText={(v) => { setPassword(v); setLoginError(null); }}
                    editable={!isAnyPending && !mfaChallenge}
                    secureTextEntry
                    autoComplete="current-password"
                  />
                </>
              )}
              {mfaChallenge && <View>
                <Text style={styles.heroTagline}>{prompt.title}</Text>
                <TextInput accessibilityLabel={prompt.label}
                  placeholder={prompt.placeholder}
                  keyboardType={recoveryMode ? "default" : "number-pad"} style={styles.input} value={mfaCode}
                  onChangeText={setMfaCode} maxLength={32} autoCapitalize="none" autoCorrect={false}
                  autoComplete="one-time-code" editable={!isEmailAuthPending} />
                <Pressable accessibilityRole="button" disabled={isEmailAuthPending} onPress={() => { setRecoveryMode(!recoveryMode); setMfaCode(""); }}>
                  <Text style={styles.heroTagline}>{prompt.toggle}</Text>
                </Pressable>
                <Pressable accessibilityRole="button" disabled={isEmailAuthPending} onPress={() => { setMfaProvider(null); setRecoveryMode(false); setMfaChallenge(null); setMfaCode(""); setPassword(""); }}>
                  <Text style={styles.heroTagline}>Back to sign-in</Text>
                </Pressable>
              </View>}
              {/* DESIGN.md §Buttons: primary CTA uses `nyx-gradient-vivid`. */}
              <Pressable
                onPress={() => void handleEmailLogin()}
                accessibilityRole="button"
                disabled={isAnyPending || (mfaChallenge ? !/^(?:\d{6}|[A-Za-z0-9_-]{8}|[a-fA-F0-9]{32})$/.test(mfaCode) : !email.trim() || !password)}
                style={[styles.signInButtonWrap, (isEmailAuthPending || (mfaChallenge ? !mfaCode : !email.trim() || !password)) && styles.buttonDisabled]}
              >
                <ExpoLinearGradient
                  colors={[colors.gradientStart, colors.gradientEnd]}
                  start={{ x: 0, y: 0 }}
                  end={{ x: 1, y: 0 }}
                  style={styles.signInButton}
                >
                  <View style={styles.socialAuthContent}>
                    {isEmailAuthPending ? (
                      <ActivityIndicator size="small" color={colors.onPrimary} />
                    ) : null}
                    <Text style={styles.signInButtonText}>{isEmailAuthPending ? "Signing in..." : mfaChallenge ? "Verify & sign in" : "Sign In"}</Text>
                  </View>
                </ExpoLinearGradient>
              </Pressable>
            </>

          {loginConfig?.emailAuthEnabled && !mfaChallenge ? (
            <Pressable
              accessibilityRole="link"
              accessibilityLabel="Create account"
              disabled={isAnyPending}
              onPress={() => void openRegistration()}
              style={styles.createAccountLink}
            >
              <Text style={styles.legalLink}>Create account</Text>
            </Pressable>
          ) : null}

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
                  Registration is invite-only.
                  {NYXID_FRONTEND_URL ? (
                    <>
                      {" "}
                      <Text
                        style={styles.errorLink}
                        onPress={() => void Linking.openURL(NYXID_FRONTEND_URL)}
                      >
                        Join the waitlist
                      </Text>
                      {" "}to get access.
                    </>
                  ) : null}
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

const createStyles = (c: ThemeColors) =>
  StyleSheet.create({
    scrollContentExtra: {
      paddingBottom: spacing.xxxl,
    },
    heroWrap: {
      alignItems: "center",
      gap: spacing.sm,
      marginBottom: spacing.sm,
      paddingTop: spacing.huge,
    },
    // DESIGN.md §PageHeader: mobile page titles ship at text-[22px] font-bold.
    heroTitle: {
      ...typeScale.pageHeader,
      color: c.textPrimary,
      marginTop: spacing.sm,
    },
    heroTagline: {
      ...typeScale.caption,
      color: c.textMuted,
      textAlign: "center",
    },
    emailHeading: {
      ...typeScale.bodyStrong,
      color: c.textPrimary,
    },
    // DESIGN.md §Inputs: rounded-lg (8px) with 50%-opacity chrome.
    input: {
      backgroundColor: c.cardSoft,
      borderColor: c.borderSoft,
      borderWidth: 1,
      borderRadius: radius.md,
      paddingVertical: spacing.md,
      paddingHorizontal: spacing.lg,
      color: c.textPrimary,
      ...typeScale.description,
    },
    signInButtonWrap: {
      borderRadius: radius.md,
      overflow: "hidden",
      // Soft purple ambient matching the web `shadow-[0_0_12px_rgba(90,42,241,0.25)]`.
      shadowColor: c.primary,
      shadowOffset: { width: 0, height: 4 },
      shadowOpacity: 0.3,
      shadowRadius: 12,
      elevation: 6,
    },
    signInButton: {
      borderRadius: radius.md,
      paddingVertical: spacing.lg,
      minHeight: 44,
      alignItems: "center",
      justifyContent: "center",
    },
    signInButtonText: {
      color: c.onPrimary,
      ...typeScale.label,
    },
    buttonDisabled: {
      opacity: 0.5,
    },
    createAccountLink: {
      minHeight: 44,
      alignItems: "center",
      justifyContent: "center",
    },
    // DESIGN.md §Banners & callouts: rounded-xl with theme-aware destructive tint.
    errorBanner: {
      backgroundColor: c.dangerTone.bg,
      borderWidth: 1,
      borderColor: c.dangerTone.border,
      borderRadius: radius.lg,
      paddingVertical: spacing.lg,
      paddingHorizontal: spacing.lg,
    },
    errorText: {
      color: c.danger,
      ...typeScale.caption,
      lineHeight: 18,
    },
    errorLink: {
      color: c.primaryOnTint,
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
      ...typeScale.small,
      marginHorizontal: spacing.sm,
    },
    socialAuthButton: {
      minHeight: 46,
      width: "100%",
      flexDirection: "row",
      gap: spacing.lg,
      backgroundColor: c.bg,
      borderColor: c.borderSoft,
      borderWidth: 1,
      borderRadius: radius.md,
      paddingVertical: spacing.sm,
      paddingHorizontal: spacing.xxl,
      alignItems: "center",
      justifyContent: "center",
    },
    socialAuthButtonPressed: {
      backgroundColor: c.navActive,
    },
    socialAuthButtonDisabled: {
      opacity: 0.6,
    },
    providerIcon: {
      width: 28,
      height: 28,
      borderRadius: radius.sm,
      backgroundColor: c.navActive,
      alignItems: "center",
      justifyContent: "center",
    },
    providerLabel: {
      flex: 1,
      color: c.textPrimary,
      ...typeScale.label,
      textAlign: "left",
    },
    socialAuthContent: {
      flexDirection: "row",
      alignItems: "center",
      justifyContent: "center",
      gap: spacing.sm,
    },
    socialAuthText: {
      color: c.textPrimary,
      ...typeScale.label,
    },
    legal: {
      color: c.textMuted,
      ...typeScale.small,
      marginTop: spacing.sm,
      textAlign: "center",
    },
    legalLink: {
      color: c.textSecondary,
      ...typeScale.small,
      textDecorationLine: "underline",
    },
  });
