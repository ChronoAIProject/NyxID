import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import FontAwesome from "@expo/vector-icons/FontAwesome";
import * as WebBrowser from "expo-web-browser";
import { NativeStackScreenProps } from "@react-navigation/native-stack";
import { ActivityIndicator, Linking, Pressable, ScrollView, StyleSheet, Text, TextInput, View } from "react-native";
import Svg, { Path, Defs, LinearGradient, Stop, Circle as SvgCircle } from "react-native-svg";
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

// Ambient backdrop for the auth hero: three concentric rings with
// rotating linear-gradient strokes (re-using the brand orbital motif
// from NyxFAB) plus a soft purple wash. Pure decoration — sits behind
// the logo, ignores touches.
function PortalAmbient({ size = 380 }: { size?: number }) {
  return (
    <Svg
      width={size}
      height={size}
      viewBox={`0 0 ${size} ${size}`}
      pointerEvents="none"
    >
      <Defs>
        <LinearGradient id="amb-ring1" gradientUnits="userSpaceOnUse" x1="0" y1={size / 2} x2={size} y2={size / 2}>
          <Stop offset="0" stopColor="#A78BFA" stopOpacity={0.55} />
          <Stop offset="0.5" stopColor="#A78BFA" stopOpacity={0} />
        </LinearGradient>
        <LinearGradient id="amb-ring2" gradientUnits="userSpaceOnUse" x1="0" y1={size / 2} x2={size} y2={size / 2} gradientTransform={`rotate(120 ${size / 2} ${size / 2})`}>
          <Stop offset="0" stopColor="#C4B5FD" stopOpacity={0.4} />
          <Stop offset="0.5" stopColor="#C4B5FD" stopOpacity={0} />
        </LinearGradient>
        <LinearGradient id="amb-ring3" gradientUnits="userSpaceOnUse" x1="0" y1={size / 2} x2={size} y2={size / 2} gradientTransform={`rotate(240 ${size / 2} ${size / 2})`}>
          <Stop offset="0" stopColor="#DDD6FE" stopOpacity={0.3} />
          <Stop offset="0.5" stopColor="#DDD6FE" stopOpacity={0} />
        </LinearGradient>
      </Defs>
      <SvgCircle cx={size / 2} cy={size / 2} r={size / 2 - 4} fill="none" stroke="url(#amb-ring1)" strokeWidth={1} />
      <SvgCircle cx={size / 2} cy={size / 2} r={size / 2 - 60} fill="none" stroke="url(#amb-ring2)" strokeWidth={1} />
      <SvgCircle cx={size / 2} cy={size / 2} r={size / 2 - 110} fill="none" stroke="url(#amb-ring3)" strokeWidth={0.8} />
      <SvgCircle cx={size * 0.18} cy={size * 0.32} r={2} fill="#C4B5FD" opacity={0.5} />
      <SvgCircle cx={size * 0.84} cy={size * 0.58} r={1.5} fill="#C4B5FD" opacity={0.4} />
      <SvgCircle cx={size * 0.72} cy={size * 0.22} r={1} fill="#C4B5FD" opacity={0.6} />
      <SvgCircle cx={size * 0.28} cy={size * 0.78} r={1} fill="#C4B5FD" opacity={0.35} />
    </Svg>
  );
}

// Inline render of mobile/assets/sources/app_icon.svg
// (kept in sync by hand — if the source changes, regenerate the PNG icons
// AND update this JSX so the login screen matches the launcher icon)
function PortalMarkLogo({ size = 96 }: { size?: number }) {
  return (
    <Svg width={size} height={size} viewBox="0 0 702 702" fill="none">
      <Defs>
        <LinearGradient id="ai_bg" x1="351" y1="0" x2="351" y2="702" gradientUnits="userSpaceOnUse">
          <Stop offset="0" stopColor="#221250" />
          <Stop offset="1" stopColor="#070707" />
        </LinearGradient>
        <LinearGradient id="ai_n" x1="351" y1="140.062" x2="351" y2="561.933" gradientUnits="userSpaceOnUse">
          <Stop offset="0" stopColor="#A672FB" />
          <Stop offset="1" stopColor="#5E00F5" />
        </LinearGradient>
        <LinearGradient id="ai_hl" x1="351" y1="140.062" x2="351" y2="561.933" gradientUnits="userSpaceOnUse">
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
          <View style={styles.heroAmbient} pointerEvents="none">
            <View style={styles.heroGlow} />
            <PortalAmbient />
          </View>
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
                    onPress={() => void Linking.openURL("https://nyx.chrono-ai.fun")}
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
    position: "relative",
  },
  heroAmbient: {
    position: "absolute",
    top: -40,
    width: 380,
    height: 380,
    alignItems: "center",
    justifyContent: "center",
  },
  heroGlow: {
    position: "absolute",
    width: 240,
    height: 240,
    borderRadius: radius.full,
    backgroundColor: c.primaryGlow,
    opacity: 0.9,
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
    ...typeScale.description,
  },
  signInButton: {
    backgroundColor: c.primary,
    borderRadius: radius.md,
    paddingVertical: spacing.lg,
    minHeight: 44,
    alignItems: "center",
    justifyContent: "center",
    // Matches PrimaryButton — brand CTAs carry a soft purple ambient.
    shadowColor: c.primary,
    shadowOffset: { width: 0, height: 6 },
    shadowOpacity: 0.35,
    shadowRadius: 16,
    elevation: 6,
  },
  signInButtonText: {
    color: c.onPrimary,
    ...typeScale.label,
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
    ...typeScale.small,
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
