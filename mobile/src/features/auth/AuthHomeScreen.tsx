import { localizedToast } from "../../i18n/localizedToast";
import { useLocalizedState } from "../../i18n/useLocalizedState";
import { Translation } from "../../i18n/Translation";
import { useAppLanguage } from "../../i18n/LanguageProvider";
import { translateApp } from "../../i18n/appTranslation";
import { mfaPrompt } from "./mfaPrompt";
import { accountWebUrl } from "../account/accountWebHandoff";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import * as WebBrowser from "expo-web-browser";
import { useIsFocused } from "@react-navigation/native";
import { useQuery } from "@tanstack/react-query";
import { NativeStackScreenProps } from "@react-navigation/native-stack";
import {
  ActivityIndicator,
  AccessibilityInfo,
  AppState,
  Linking,
  Platform,
  Pressable,
  StyleSheet,
  Text,
  View,
} from "react-native";
import Svg, { Path } from "react-native-svg";
import { AuthSubmitButton } from "./AuthSubmitButton";
import type { RootStackParamList } from "../../app/AppNavigator";

import { ScreenContainer } from "../../components/ScreenContainer";
import { ToastKind, ToastOverlay, ToastState } from "../../components/ToastOverlay";
import { runAuthAttempt } from "../../lib/auth/authAttempt";
import { MfaChallengeError } from "../../lib/auth/mfaChallenge";
import { ApiError } from "../../lib/api/ApiError";
import { authSessionGeneration } from "../../lib/auth/sessionStore";
import { mobileApi } from "../../lib/api/mobileApi";
import { resolveErrorMessage } from "../../lib/api/errorMessages";
import { useAuthSession } from "./AuthSessionContext";
import { getPublicAuthConfig } from "../../lib/api/publicAuthApi";
import { requireSocialProvider } from "./publicAuthConfig";
import {
  parseSocialCallback,
  SOCIAL_CALLBACK_URL,
} from "./socialCallback";
import { socialProviders, type SocialProvider } from "../../lib/auth/socialProviders";
import { capture } from "../../lib/telemetry";
import { getFrontendUrl } from "../../lib/env";
import { useAuthTheme } from "./useAuthTheme";
import { AuthTextInput } from "./AuthTextInput";
import type { ThemeColors } from "../../theme/mobileTheme";
import { createFlowStyles } from "../../theme/flowStyles";
import { BrandNebula, BreathingMark, RiseIn } from "../../components/brand/BrandNebula";
import { Eye, EyeOff, LockKeyhole, Mail } from "lucide-react-native";
import { PortalMarkLogo } from "../../components/brand/PortalMark";
import { KeyboardScrollView, type KeyboardScrollViewHandle } from "../../components/workspace/KeyboardScrollView";
import { fonts, radius, spacing, typeScale } from "../../theme/designTokens";

type Props = NativeStackScreenProps<RootStackParamList, "Auth">;

function resolveAuthError(error: unknown): string {
  if (!(error instanceof Error)) return translateApp("Sign-in failed. Please try again.");
  return error.message || translateApp("Sign-in failed. Please try again.");
}

function resolveSocialAuthError(error: string | undefined): string {
  switch (error) {
    case "social_auth_invalid_mfa":
      return translateApp(
        "Your verification challenge is invalid. Please sign in again.",
      );
    case "social_auth_denied":
      return translateApp("Social sign-in was cancelled.");
    case "social_auth_csrf":
      return translateApp(
        "Social sign-in failed security check. Please retry.",
      );
    case "social_auth_conflict":
      return translateApp("This email is linked to another login method.");
    case "social_auth_no_email":
      return translateApp("Provider did not return a verified email.");
    case "social_auth_deactivated":
      return translateApp("This account is deactivated.");
    case "social_auth_exchange":
    case "social_auth_profile":
      return translateApp("Unable to complete social sign-in.");
    case "social_auth_registration_closed":
      return "WAITLIST";
    default:
      return error || translateApp("Social sign-in failed. Please try again.");
  }
}

// Inline render of mobile/assets/sources/app_icon.svg
// (kept in sync by hand — if the source changes, regenerate the PNG icons
// AND update this JSX so the login screen matches the launcher icon)
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
  useAppLanguage();
  const { colors, mode } = useAuthTheme();
  const styles = useMemo(() => createStyles(colors, mode), [colors, mode]);
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
        pressed && !disabled && styles.socialAuthButtonPressed,
        pressed && !disabled && styles.pressed,
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
        {loading ? translateApp("Connecting…") : label}
      </Text>
    </Pressable>
  );
}

// Same provider marks as the web sign-in rows.
function SocialProviderIcon({ provider, color }: { provider: SocialProvider; color: string;
}) {
  useAppLanguage();
  return (
    <Svg width={16} height={16} viewBox="0 0 24 24" fill={color} accessible={false}>
      {provider === "google" ? (
        <>
        <Path d="M22.56 12.25c0-.78-.07-1.53-.2-2.25H12v4.26h5.92a5.06 5.06 0 01-2.2 3.32v2.77h3.57c2.08-1.92 3.28-4.74 3.28-8.1z" fill="#4285F4" />
        <Path d="M12 23c2.97 0 5.46-.98 7.28-2.66l-3.57-2.77c-.98.66-2.23 1.06-3.71 1.06-2.86 0-5.29-1.93-6.16-4.53H2.18v2.84C3.99 20.53 7.7 23 12 23z" fill="#34A853" />
        <Path d="M5.84 14.09c-.22-.66-.35-1.36-.35-2.09s.13-1.43.35-2.09V7.07H2.18C1.43 8.55 1 10.22 1 12s.43 3.45 1.18 4.93l2.85-2.22.81-.62z" fill="#FBBC05" />
        <Path d="M12 5.38c1.62 0 3.06.56 4.21 1.64l3.15-3.15C17.45 2.09 14.97 1 12 1 7.7 1 3.99 3.47 2.18 7.07l3.66 2.84c.87-2.6 3.3-4.53 6.16-4.53z" fill="#EA4335" />
      </>
      ) : provider === "github" ? (
        <Path d="M12 0c-6.626 0-12 5.373-12 12 0 5.302 3.438 9.8 8.207 11.387.599.111.793-.261.793-.577v-2.234c-3.338.726-4.033-1.416-4.033-1.416-.546-1.387-1.333-1.756-1.333-1.756-1.089-.745.083-.729.083-.729 1.205.084 1.839 1.237 1.839 1.237 1.07 1.834 2.807 1.304 3.492.997.107-.775.418-1.305.762-1.604-2.665-.305-5.467-1.334-5.467-5.931 0-1.311.469-2.381 1.236-3.221-.124-.303-.535-1.524.117-3.176 0 0 1.008-.322 3.301 1.23.957-.266 1.983-.399 3.003-.404 1.02.005 2.047.138 3.006.404 2.291-1.552 3.297-1.23 3.297-1.23.653 1.653.242 2.874.118 3.176.77.84 1.235 1.911 1.235 3.221 0 4.609-2.807 5.624-5.479 5.921.43.372.823 1.102.823 2.222v3.293c0 .319.192.694.801.576 4.765-1.589 8.199-6.086 8.199-11.386 0-6.627-5.373-12-12-12z" />
      ) : (
        <Path d="M17.05 20.28c-.98.95-2.05.88-3.08.4-1.09-.5-2.08-.48-3.24 0-1.44.62-2.2.44-3.06-.4C2.79 15.25 3.51 7.59 9.05 7.31c1.35.07 2.29.74 3.08.8 1.18-.24 2.31-.93 3.57-.84 1.51.12 2.65.72 3.4 1.8-3.12 1.87-2.38 5.98.48 7.13-.57 1.5-1.31 2.99-2.54 4.09zM12.03 7.25c-.15-2.23 1.66-4.07 3.74-4.25.29 2.58-2.34 4.5-3.74 4.25z" />
      )}
    </Svg>
  );
}

export function AuthHomeScreen({ navigation }: Props) {
  useAppLanguage();
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
  const { colors, mode } = useAuthTheme();
  const styles = useMemo(() => createStyles(colors, mode), [colors, mode]);
  const flowStyles = useMemo(() => createFlowStyles(colors), [colors]);
  const [isSocialAuthPending, setIsSocialAuthPending] = useState(false);
  const [pendingSocialProvider, setPendingSocialProvider] =
    useState<SocialProvider | null>(null);
  const [toast, setToast] = useState<ToastState | null>(null);
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [showPassword, setShowPassword] = useState(false);
  const [mfaProvider, setMfaProvider] = useState<string | null>(null);
  const [recoveryMode, setRecoveryMode] = useState(false);
  const [mfaChallenge, setMfaChallenge] = useState<MfaChallengeError | null>(null);
  const [mfaCode, setMfaCode] = useState("");
  const [isEmailAuthPending, setIsEmailAuthPending] = useState(false);
  const socialLoginLock = useRef(false);
  const [loginError, setLoginError] = useLocalizedState<string | null>(null);
  const passwordInvalid = loginError === "INCORRECT_LOGIN_DETAILS";
  const loginErrorMessage = passwordInvalid
    ? translateApp("Incorrect login details.")
    : loginError;
  const [registrationNotice, setRegistrationNotice] = useLocalizedState<string | null>(null);
  const [registrationPending, setRegistrationPending] = useState(false);
  const registrationLock = useRef(false);
  const { signInWithSession } = useAuthSession();
  const isMountedRef = useRef(true);
  const emailLoginLock = useRef(false);
  const lastHandledSocialUrlRef = useRef<string | null>(null);
  const scrollController = useRef<KeyboardScrollViewHandle | null>(null);
  const errorBannerRef = useRef<View>(null);

  useEffect(() => {
    if (loginErrorMessage && Platform.OS === "ios") {
      AccessibilityInfo.announceForAccessibility(loginErrorMessage === "WAITLIST"
        ? translateApp("Registration is invite-only.") : loginErrorMessage);
    }
  }, [loginErrorMessage]);

  const showToast = (message: () => string, kind: ToastKind) => {
    setToast(localizedToast(() => ({ message: message(), kind })));
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
        setLoginError(() => (resolveSocialAuthError(callback.error)));
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
        setLoginError(() => (translateApp("Missing social auth access token.")));
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
        setLoginError(() => (resolveErrorMessage(error)));
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
      // The query owns discovery errors and its retry UI. Copying one into
      // loginError would leave a stale banner after configuration recovers.
      if (available.error) return;
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
        setLoginError(
          () => (translateApp("Sign-in was closed ({{type}}). Please try again.", {
            type: result.type,
          })),
        );
        return;
      }

      setLoginError(
        () => (translateApp("Social sign-in failed: {{type}}", {
          type: result.type,
        })),
      );
    } catch (error) {
      setLoginError(() => (resolveErrorMessage(error)));
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
          // A newer sign-in or sign-out replaced this attempt; it owns the screen.
          if (
            error instanceof Error &&
            error.message === "auth_session_changed"
          )
            return;
          if (error instanceof MfaChallengeError) {
            setMfaProvider(null); setRecoveryMode(false); setMfaChallenge(error); setPassword(""); setMfaCode("");
          } else {
            setMfaCode("");
            if (error instanceof ApiError && error.reason === "mfa_challenge_expired") {
              setMfaProvider(null); setRecoveryMode(false); setMfaChallenge(null);
              setLoginError(
                () => (translateApp(
                  "Your verification expired. Please sign in again.",
                )),
              );
            } else if (!mfaChallenge && error instanceof ApiError && (
              error.errorCode === 2000 || error.errorKey === "authentication_failed" ||
              error.errorCode === 1008 || error.errorKey === "validation_error"
            )) {
              setLoginError(() => "INCORRECT_LOGIN_DETAILS");
            } else { setLoginError(() => (resolveErrorMessage(error))); }
          }
        },
        () => isMountedRef.current,
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
  const signInBlocked = isAnyPending || (mfaChallenge
    ? !/^(?:\d{6}|[A-Za-z0-9_-]{8}|[a-fA-F0-9]{32})$/.test(mfaCode)
    : !email.trim() || !password);

  const openRegistration = async () => {
    if (!loginConfig?.emailAuthEnabled || isAnyPending || mfaChallenge || registrationPending || registrationLock.current) return;
    registrationLock.current = true;
    setRegistrationPending(true);
    setRegistrationNotice(
      () => (translateApp(
        "Registration opens on the NyxID website. Close the browser to return here and sign in after verifying your email.",
      )),
    );
    try {
      await WebBrowser.openBrowserAsync(
        accountWebUrl(getFrontendUrl(), "/register"),
      );
      if (isMountedRef.current) setRegistrationNotice(
          () => (translateApp(
            "Returned from registration. Registration is not verified by closing the browser. Complete any email verification, then sign in below.",
          )),
        );
    } catch (error) {
      setLoginError(() => (resolveAuthError(error)));
    } finally {
      registrationLock.current = false;
      if (isMountedRef.current) setRegistrationPending(false);
    }
  };

  return (
    <ScreenContainer>
      <BrandNebula />
      <KeyboardScrollView
        revealController={scrollController}
        style={flowStyles.content}
        contentContainerStyle={[
          flowStyles.scrollContent,
          styles.scrollContentExtra,
          { paddingHorizontal: spacing.xxl },
        ]}
        showsVerticalScrollIndicator={false}
        keyboardShouldPersistTaps="handled"
        keyboardDismissMode={Platform.OS === "ios" ? "interactive" : "none"}
      >
        {/* Hero: the portal mark on a faint halo, rising in on arrival. */}
        <RiseIn>
        <View style={styles.heroWrap}>
          <BreathingMark>
            <PortalMarkLogo size={76} />
          </BreathingMark>
          <Text accessibilityRole="header" style={styles.heroTitle}><Translation message="Welcome to {{brand}}" values={{ brand: <Text style={styles.heroBrand}>{"NyxID"}</Text> }} />
          </Text>
          <Text style={styles.heroLead}>{translateApp("Your agents, connections and approvals,\nin one place.")}</Text>
        </View>
        </RiseIn>

        {/* Social providers follow server config; existing accounts can always sign in. */}
        <RiseIn delay={140}>
        <View style={styles.panel}>
          {/* Social login */}
          {publicConfig.isPending ? (
            <View style={styles.socialAuthContent}>
              <ActivityIndicator size="small" color={colors.primary} />
              <Text
                style={{ ...typeScale.caption, color: colors.textSecondary }}
              >
                  {translateApp("Loading sign-in options...")}
                </Text>
            </View>
          ) : publicConfig.isError ? (
            <View style={{ gap: spacing.md }}>
              <Text
                style={{ ...typeScale.caption, color: colors.textSecondary }}
              >
                  {translateApp(
                    "Could not load sign-in options. Please try again.",
                  )}
                </Text>
              <Pressable
                accessibilityRole="button"
                disabled={publicConfig.isFetching}
                onPress={() => void publicConfig.refetch()}
                style={styles.socialAuthButton}
              >
                <Text style={styles.socialAuthText}>
                  {publicConfig.isFetching
                    ? translateApp("Retrying...")
                      : translateApp("Retry sign-in options")}
                </Text>
              </Pressable>
            </View>
          ) : null}
          {socialProviders
            .filter(({ id }) => loginConfig?.socialProviders.includes(id))
            .map(({ id, signInLabel }) => (
              <SocialAuthButton
                key={id}
                label={translateApp(signInLabel)}
                provider={id}
                disabled={isAnyPending}
                loading={isSocialAuthPending && pendingSocialProvider === id}
                onPress={() => void startSocialLogin(id)}
              />
            ))}

            <>
              {loginConfig && loginConfig.socialProviders.length > 0 ? (
                <View style={styles.dividerRow}>
                  <View style={styles.dividerLine} />
                  <Text style={styles.dividerText}>
                    {translateApp("or continue with email")}
                  </Text>
                  <View style={styles.dividerLine} />
                </View>
              ) : null}
              {prompt.showPasswordFields && (
                <>
                  <AuthTextInput
                    frameStyle={styles.field}
                    leading={<Mail size={18} color={colors.textMuted} />}
                    style={styles.input}
                    accessibilityLabel={translateApp("Email")}
                    placeholder={translateApp("Email")}
                    placeholderTextColor={colors.textMuted}
                    value={email}
                    onChangeText={(v) => { setEmail(v); setMfaChallenge(null); setMfaCode(""); setLoginError(null); }}
                    editable={!isAnyPending && !mfaChallenge}
                    keyboardType="email-address"
                    autoCapitalize="none"
                    autoComplete="email"
                  />
                  <AuthTextInput
                    frameStyle={passwordInvalid ? [styles.field, styles.invalidField] : styles.field}
                    invalid={passwordInvalid}
                    leading={<LockKeyhole size={18} color={colors.textMuted} />}
                    style={styles.input}
                    accessibilityLabel={translateApp("Password")}
                    placeholder={translateApp("Password")}
                    placeholderTextColor={colors.textMuted}
                    value={password}
                    onChangeText={(v) => { setPassword(v); setLoginError(null); }}
                    editable={!isAnyPending && !mfaChallenge}
                    secureTextEntry={!showPassword}
                    autoComplete="current-password"
                    trailing={<Pressable
                    accessibilityRole="button"
                    accessibilityLabel={showPassword ? translateApp("Hide password")
                            : translateApp("Show password")
                        }
                    hitSlop={8}
                    onPress={() => setShowPassword((value) => !value)}
                    style={styles.fieldAction}
                  >
                    {showPassword ? (
                      <EyeOff size={18} color={colors.textSecondary} />
                    ) : (
                      <Eye size={18} color={colors.textSecondary} />
                    )}
                  </Pressable>}
                  />
                </>
              )}
              {mfaChallenge && (
                <View>
                <Text style={styles.heroTagline}>{prompt.title}</Text>
                <AuthTextInput frameStyle={mode === "light" ? styles.field : undefined} accessibilityLabel={prompt.label}
                  placeholder={prompt.placeholder}
                  keyboardType={recoveryMode ? "default" : "number-pad"} style={styles.input} value={mfaCode}
                  onChangeText={setMfaCode} maxLength={32} autoCapitalize="none" autoCorrect={false}
                  autoComplete="one-time-code" editable={!isEmailAuthPending} />
                <Pressable accessibilityRole="button" disabled={isEmailAuthPending} onPress={() => { setRecoveryMode(!recoveryMode); setMfaCode(""); }}>
                  <Text style={styles.heroTagline}>{prompt.toggle}</Text>
                </Pressable>
                <Pressable accessibilityRole="button" disabled={isEmailAuthPending} onPress={() => { setMfaProvider(null); setRecoveryMode(false); setMfaChallenge(null); setMfaCode(""); setPassword(""); }}>
                  <Text style={styles.heroTagline}>
                      {translateApp("Back to sign-in")}
                    </Text>
                </Pressable>
              </View>
              )}
        {loginError && (
          <View ref={errorBannerRef} style={passwordInvalid ? styles.credentialError : styles.errorBanner}
            accessibilityRole="alert" accessibilityLiveRegion="assertive"
            onLayout={() => scrollController.current?.reveal(errorBannerRef.current)}>
            {loginError === "WAITLIST" ? (
              <>
                <Text style={styles.errorText}>
                  {getFrontendUrl() ? <Translation
                    message="Registration is invite-only. {{waitlist}} to get access."
                    values={{ waitlist: <Text style={styles.errorLink} onPress={() => void Linking.openURL(getFrontendUrl())}>{translateApp("Join the waitlist")}</Text> }}
                  /> : translateApp("Registration is invite-only.")}
                </Text>
              </>
            ) : (
              <Text style={styles.errorText}>{loginErrorMessage}</Text>
            )}
          </View>
        )}
              <AuthSubmitButton
                onPress={() => void handleEmailLogin()}
                disabled={signInBlocked}
                loading={isEmailAuthPending}
                style={{ marginTop: 4 }}
                label={isEmailAuthPending ? translateApp("Signing in...")
                    : mfaChallenge ? translateApp("Verify & sign in")
                      : translateApp("Sign In")
                }
              />
            </>

          {loginConfig?.emailAuthEnabled && !mfaChallenge ? (
            <Pressable
              accessibilityRole="link"
              accessibilityLabel={translateApp("Create account")}
                disabled={isAnyPending}
              onPress={() => void openRegistration()}
              style={styles.createAccountLink}
            >
              <Text style={styles.legalLink}>
                  {translateApp("Create account")}
                </Text>
            </Pressable>
          ) : null}

          {/* Legal */}
          <Text style={styles.legal}><Translation message="By continuing, you agree to {{terms}} and {{privacy}}." values={{ terms: <Text
              style={styles.legalLink}
              onPress={() => {
                capture({
                  name: "ui.mobile_legal_page_opened",
                  props: { page: "terms" },
                });
                navigation.navigate("TermsOfService");
              }}
            >
                {translateApp("Terms")}
              </Text>, privacy: <Text
              style={styles.legalLink}
              onPress={() => {
                capture({
                  name: "ui.mobile_legal_page_opened",
                  props: { page: "privacy" },
                });
                navigation.navigate("PrivacyPolicy");
              }}
            >
                {translateApp("Privacy")}
              </Text> }} /></Text>
        </View>
        </RiseIn>

        {registrationNotice ? (
          <Text accessibilityRole="alert" style={{ color: colors.textSecondary }}>{registrationNotice}</Text>
        ) : null}
      </KeyboardScrollView>
      <ToastOverlay toast={toast} bottom={64} />
    </ScreenContainer>
  );
}

const createStyles = (c: ThemeColors, mode: string) =>
  StyleSheet.create({
    scrollContentExtra: {
      paddingBottom: spacing.xxxl,
      width: "100%",
      maxWidth: 560,
      alignSelf: "center",
    },
    heroWrap: {
      alignItems: "center",
      gap: 10,
      marginBottom: 28,
      paddingTop: 72,
    },
    heroTitle: {
      fontFamily: fonts.displaySemi,
      fontSize: 30,
      lineHeight: 36,
      color: c.textPrimary,
      marginTop: 18,
      textAlign: "center",
    },
    heroLead: {
      fontFamily: fonts.body,
      fontSize: 15,
      lineHeight: 21,
      color: c.textSecondary,
      textAlign: "center",
      paddingHorizontal: 12,
    },
    panel: { gap: 12 },
    heroBrand: { color: c.primaryOnTint },
    field: {
      flexDirection: "row",
      alignItems: "center",
      gap: 10,
      minHeight: 52,
      paddingLeft: 16,
      paddingRight: 6,
      borderRadius: 14,
      borderWidth: mode === "light" ? 1 : StyleSheet.hairlineWidth,
      borderColor: c.borderSoft,
      backgroundColor: c.card,
    },
    fieldAction: { width: 40, height: 40, alignItems: "center", justifyContent: "center" },
    invalidField: { borderColor: c.danger, borderWidth: 1 },
    pressed: { transform: [{ scale: 0.98 }] },
    heroTagline: {
      ...typeScale.caption,
      color: c.textMuted,
      textAlign: "center",
    },
    emailHeading: {
      ...typeScale.bodyStrong,
      color: c.textPrimary,
    },
    input: {
      flex: 1,
      minHeight: 50,
      paddingVertical: 14,
      color: c.textPrimary,
      fontFamily: fonts.body,
      fontSize: 16,
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
    credentialError: {
      paddingHorizontal: 2,
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
      marginVertical: 6,
    },
    dividerLine: {
      flex: 1,
      height: 1,
      backgroundColor: c.borderSoft,
    },
    dividerText: {
      color: c.textMuted,
      ...typeScale.small,
      marginHorizontal: spacing.sm,
    },
    // Provider rows: soft filled pills, mark and label centred together.
    socialAuthButton: {
      minHeight: 52,
      width: "100%",
      flexDirection: "row",
      gap: 10,
      backgroundColor: c.card,
      borderColor: c.borderSoft,
      borderWidth: mode === "light" ? 1 : StyleSheet.hairlineWidth,
      borderRadius: 14,
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
      width: 20,
      height: 20,
      alignItems: "center",
      justifyContent: "center",
    },
    providerLabel: {
      color: c.textPrimary,
      fontFamily: fonts.bodySemi,
      fontSize: 15,
      lineHeight: 20,
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
