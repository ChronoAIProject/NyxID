import { useCallback, useEffect, useRef, useState } from "react";
import FontAwesome from "@expo/vector-icons/FontAwesome";
import { NativeStackScreenProps } from "@react-navigation/native-stack";
import { Linking, Pressable, ScrollView, StyleSheet, Text, View } from "react-native";
import type { RootStackParamList } from "../../app/AppNavigator";
import { MobileStatusBar } from "../../components/MobileStatusBar";
import { ScreenContainer } from "../../components/ScreenContainer";
import { SectionBadge } from "../../components/SectionBadge";
import { ToastKind, ToastOverlay, ToastState } from "../../components/ToastOverlay";
import { mobileApi } from "../../lib/api/mobileApi";
import { activatePushAfterLogin } from "../../lib/notifications/pushNotifications";
import { useAuthSession } from "./AuthSessionContext";
import { mobileTheme } from "../../theme/mobileTheme";
import { flowStyles } from "../../theme/flowStyles";
import { radius, spacing, typeScale } from "../../theme/designTokens";

type SocialProvider = "google" | "github" | "apple";
type BackendSocialProvider = "google" | "github";
type Props = NativeStackScreenProps<RootStackParamList, "Auth">;
const SOCIAL_CALLBACK_URL = "nyxid://auth/social/callback";

type SocialCallback = {
  status: "success" | "error";
  accessToken?: string;
  refreshToken?: string;
  error?: string;
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
    if (statusRaw !== "success" && statusRaw !== "error") {
      return null;
    }

    if (statusRaw === "error") {
      return {
        status: "error",
        error: parsed.searchParams.get("error") ?? undefined,
      };
    }

    return {
      status: "success",
      accessToken: parsed.searchParams.get("access_token") ?? undefined,
      refreshToken: parsed.searchParams.get("refresh_token") ?? undefined,
    };
  } catch {
    return null;
  }
}

function SocialAuthButton({
  label,
  provider,
  disabled = false,
  onPress,
}: {
  label: string;
  provider: SocialProvider;
  disabled?: boolean;
  onPress: () => void;
}) {
  const iconName = provider === "google" ? "google" : provider === "github" ? "github" : "apple";
  const iconColor = "#F9FAFB";

  return (
    <Pressable onPress={onPress} disabled={disabled} style={[styles.socialAuthButton, disabled && styles.socialAuthButtonDisabled]}>
      <View style={styles.socialAuthContent}>
        <FontAwesome name={iconName} size={16} color={iconColor} />
        <Text style={styles.socialAuthText}>{label}</Text>
      </View>
    </Pressable>
  );
}

export function AuthHomeScreen({ navigation }: Props) {
  const [isSocialAuthPending, setIsSocialAuthPending] = useState(false);
  const [toast, setToast] = useState<ToastState | null>(null);
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
        return;
      }

      if (!callback.accessToken) {
        showToast("Missing social auth access token.", "error");
        return;
      }

      if (isMountedRef.current) {
        setToast(null);
        setIsSocialAuthPending(true);
      }

      try {
        await signInWithSession({
          accessToken: callback.accessToken,
          refreshToken: callback.refreshToken,
        });

        try {
          const pushResult = await activatePushAfterLogin();
          if (__DEV__) {
            console.log("[push] activate after social login", pushResult);
          }
        } catch (pushError) {
          console.warn("[push] activate after social login failed", pushError);
        }
      } catch (error) {
        showToast(resolveAuthError(error), "error");
      } finally {
        if (isMountedRef.current) {
          setIsSocialAuthPending(false);
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

    if (provider === "apple") {
      showToast("Apple sign-in is not available yet.", "info");
      return;
    }

    if (isMountedRef.current) {
      setToast(null);
      setIsSocialAuthPending(true);
    }

    try {
      const authorizeUrl = mobileApi.getSocialAuthorizeUrl(
        provider as BackendSocialProvider,
        SOCIAL_CALLBACK_URL
      );
      await Linking.openURL(authorizeUrl);
    } catch (error) {
      const message = error instanceof Error ? error.message : "Failed to start social sign-in.";
      showToast(message, "error");
    } finally {
      if (isMountedRef.current) {
        setIsSocialAuthPending(false);
      }
    }
  };

  return (
    <ScreenContainer>
      <MobileStatusBar />
      <ScrollView
        style={flowStyles.content}
        contentContainerStyle={[flowStyles.scrollContent, styles.scrollContentExtra]}
        showsVerticalScrollIndicator={false}
      >
        <SectionBadge label="SOCIAL ONLY" tone="info" />
        <Text style={flowStyles.title}>Continue to NyxID</Text>
        <Text style={flowStyles.subtitle}>Use Google, GitHub, or Apple to continue.</Text>

        <View style={flowStyles.card}>
          <SocialAuthButton
            label="Continue with Google"
            provider="google"
            disabled={isSocialAuthPending}
            onPress={() => void startSocialLogin("google")}
          />
          <SocialAuthButton
            label="Continue with GitHub"
            provider="github"
            disabled={isSocialAuthPending}
            onPress={() => void startSocialLogin("github")}
          />
          <SocialAuthButton
            label="Continue with Apple"
            provider="apple"
            disabled={isSocialAuthPending}
            onPress={() => void startSocialLogin("apple")}
          />

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
        </View>
      </ScrollView>
      <ToastOverlay toast={toast} bottom={64} />
    </ScreenContainer>
  );
}

const styles = StyleSheet.create({
  scrollContentExtra: {
    paddingBottom: spacing.xxxl,
  },
  legal: {
    color: "#6A6480",
    ...typeScale.caption,
    fontSize: 11,
    marginTop: spacing.sm,
  },
  legalLink: {
    color: "#B9B4CC",
    ...typeScale.caption,
    fontSize: 11,
    textDecorationLine: "underline",
  },
  socialAuthButton: {
    backgroundColor: "#0F1422",
    borderColor: "#263042",
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
    color: "#F8FAFC",
    ...typeScale.caption,
    fontWeight: "600",
    fontSize: 12,
  },
});
