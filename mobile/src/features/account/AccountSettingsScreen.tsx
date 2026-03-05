import { useMutation, useQueryClient } from "@tanstack/react-query";
import { NativeStackScreenProps } from "@react-navigation/native-stack";
import { Alert, ScrollView, StyleSheet, Text, View } from "react-native";
import { RootStackParamList } from "../../app/AppNavigator";
import { MobileStatusBar } from "../../components/MobileStatusBar";
import { PrimaryButton } from "../../components/PrimaryButton";
import { ScreenContainer } from "../../components/ScreenContainer";
import { SectionBadge } from "../../components/SectionBadge";
import { ToastKind, ToastOverlay, ToastState } from "../../components/ToastOverlay";
import { useAuthSession } from "../auth/AuthSessionContext";
import { mobileApi } from "../../lib/api/mobileApi";
import { mobileTheme } from "../../theme/mobileTheme";
import { flowStyles } from "../../theme/flowStyles";
import { typeScale } from "../../theme/designTokens";
import { useEffect, useState } from "react";

type Props = NativeStackScreenProps<RootStackParamList, "AccountSettings">;

function resolveDeleteAccountError(error: unknown): {
  message: string;
  shouldForceSignOut: boolean;
} {
  const raw = error instanceof Error ? error.message : "";
  const code = raw.toLowerCase();

  if (
    code.includes("auth_session_missing") ||
    code.includes("unauthorized") ||
    code.includes("invalid_token") ||
    code.includes("token_expired") ||
    code.includes("request_failed_401")
  ) {
    return {
      message: "Session expired. Please sign in again.",
      shouldForceSignOut: true,
    };
  }

  if (code.includes("user_not_found") || code.includes("not found") || code.includes("request_failed_404")) {
    return {
      message: "Account not found or already deleted.",
      shouldForceSignOut: true,
    };
  }

  if (code.includes("network request failed") || code.includes("failed to fetch")) {
    return {
      message: "Network error. Check API server and try again.",
      shouldForceSignOut: false,
    };
  }

  // In dev, surface backend message to debug
  const fallback = __DEV__ && raw ? raw : "Failed to delete account. Please try again.";
  return {
    message: fallback,
    shouldForceSignOut: false,
  };
}

export function AccountSettingsScreen({ navigation }: Props) {
  const [toast, setToast] = useState<ToastState | null>(null);
  const queryClient = useQueryClient();
  const { signOut } = useAuthSession();

  const showToast = (message: string, kind: ToastKind) => {
    setToast({ message, kind });
  };

  useEffect(() => {
    if (!toast) return;
    const timer = setTimeout(() => setToast(null), 2400);
    return () => clearTimeout(timer);
  }, [toast]);

  const deleteAccountMutation = useMutation({
    mutationFn: () => mobileApi.deleteAccount(),
    onSuccess: async () => {
      showToast("Account and server-side data deleted. You have been signed out.", "success");
      try {
        await signOut();
      } catch (error) {
        if (__DEV__) console.warn("[auth] sign out after delete failed", error);
      }
      queryClient.clear();
    },
    onError: (error) => {
      const resolved = resolveDeleteAccountError(error);
      showToast(resolved.message, "error");
      if (!resolved.shouldForceSignOut) {
        return;
      }

      void signOut()
        .then(() => {
          queryClient.clear();
        })
        .catch((signOutError) => {
          if (__DEV__) console.warn("[auth] forced sign out after delete error failed", signOutError);
        });
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
            .then(() => {
              queryClient.clear();
            })
            .catch((error) => {
              if (__DEV__) console.warn("[auth] sign out failed", error);
            });
        },
      },
    ]);
  };

  const handleDeleteAccount = () => {
    Alert.alert(
      "Delete Account",
      "This action is permanent and will permanently delete your account and server-side data.",
      [
        { text: "Cancel", style: "cancel" },
        {
          text: "Delete",
          style: "destructive",
          onPress: () => {
            setToast(null);
            deleteAccountMutation.mutate();
          },
        },
      ]
    );
  };

  return (
    <ScreenContainer>
      <MobileStatusBar />
      <ScrollView
        style={flowStyles.content}
        contentContainerStyle={flowStyles.scrollContent}
        showsVerticalScrollIndicator={false}
      >
        <SectionBadge label="ACCOUNT" tone="info" />
        <Text style={flowStyles.title}>Account & Security</Text>
        <Text style={flowStyles.subtitle}>
          Manage current device session and account lifecycle operations.
        </Text>

        <View style={flowStyles.card}>
          <Text style={flowStyles.cardTitle}>Session</Text>
          <PrimaryButton
            label="Sign Out"
            kind="ghost"
            disabled={deleteAccountMutation.isPending}
            onPress={handleSignOut}
          />
        </View>

        <View style={flowStyles.card}>
          <Text style={flowStyles.cardTitle}>Danger Zone</Text>
          <PrimaryButton
            label={deleteAccountMutation.isPending ? "Deleting..." : "Delete Account"}
            kind="danger"
            disabled={deleteAccountMutation.isPending}
            onPress={handleDeleteAccount}
          />
          <Text style={styles.warningText}>
            Deleting your account is permanent and removes your account plus server-side data.
          </Text>
        </View>
      </ScrollView>
      <ToastOverlay toast={toast} />
    </ScreenContainer>
  );
}

const styles = StyleSheet.create({
  warningText: {
    color: mobileTheme.textMuted,
    ...typeScale.caption,
  },
});
