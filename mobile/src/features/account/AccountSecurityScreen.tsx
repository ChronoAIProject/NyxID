import { useAppLanguage } from "../../i18n/LanguageProvider";
import { useCallback, useEffect, useRef, useState } from "react";
import { useFocusEffect } from "@react-navigation/native";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Alert, AppState, Linking, Share } from "react-native";
import * as Clipboard from "expo-clipboard";
import { useNavigation } from "@react-navigation/native";
import type { NativeStackNavigationProp } from "@react-navigation/native-stack";
import type { WorkspaceSheetParamList } from "../../app/navigationTypes";
import { useWorkspaceBack } from "../../app/WorkspaceHeader";
import {
  SettingsGroup,
  SettingsPage,
  SettingsRow,
  SettingsTextField,
} from "../../components/settings/SettingsList";
import { SkeletonRows } from "../../components/Skeleton";
import { Notice } from "../../components/workspace/WorkspaceUI";
import { useNetworkStatus } from "../../hooks/useNetworkStatus";
import { accountApi } from "../../lib/api/accountApi";
import {
  totpCodeSchema,
  type MfaEnrollment,
} from "../../lib/api/accountSchema";
import { isApiError } from "../../lib/api/ApiError";
import { resolveErrorMessage } from "../../lib/api/errorMessages";
import { getAuthEpoch } from "../../lib/auth/authEpoch";
import { createMfaRecoveryLifecycle } from "./mfaRecoveryLifecycle";
import { AccountSecurityControls } from "./AccountSecurityControls";
import { AccountAuthorizations } from "./AccountAuthorizations";
import { AccountOrganizations } from "./AccountOrganizations";
import { sessionDeviceLabel } from "./sessionDevice";
import { formatSessionDate } from "./SessionDetailScreen";
import { disableMfaSchema } from "../../lib/api/accountSecurityTransport";

type PrivateAction = "setup" | "confirm" | "disable" | "copy" | "share";
type Feedback = { message: string; tone: "error" | "info" | "success" };

function accountError(error: unknown): string {
  if (isApiError(error) && error.errorCode === 2000)
    return "The password or authenticator code is incorrect. Please try again.";
  return resolveErrorMessage(error);
}

export function AccountSecurityScreen() {
  const { t } = useAppLanguage();
  const navigation =
    useNavigation<NativeStackNavigationProp<WorkspaceSheetParamList>>();
  const { isConnected } = useNetworkStatus();
  const queryClient = useQueryClient();
  const profile = useQuery({
    queryKey: ["account", "profile"],
    queryFn: ({ signal }) => accountApi.getProfile(signal),
  });
  const sessions = useQuery({
    queryKey: ["account", "sessions"],
    queryFn: ({ signal }) => accountApi.listSessions(signal),
  });
  const [feedback, setFeedback] = useState<Feedback | null>(null);

  // MFA secrets live only in this screen's transient state, never Query's
  // mutation cache, SecureStore, route params, or telemetry.
  const [enrollment, setEnrollment] = useState<MfaEnrollment | null>(null);
  const [enrollmentPending, setEnrollmentPending] = useState(false);
  const [totpCode, setTotpCode] = useState("");
  const [recoveryLifecycle] = useState(() =>
    createMfaRecoveryLifecycle(getAuthEpoch),
  );
  const [, setRecoveryVersion] = useState(0);
  const syncRecovery = useCallback(
    () => setRecoveryVersion((version) => version + 1),
    [],
  );
  const recovery = recoveryLifecycle.snapshot();
  const recoveryCodes = recovery.codes;
  const [revealCodes, setRevealCodes] = useState(false);
  const [disableOpen, setDisableOpen] = useState(false);
  const [password, setPassword] = useState("");
  const [disableCode, setDisableCode] = useState("");
  const [privateAction, setPrivateAction] = useState<PrivateAction | null>(
    null,
  );
  const privateBusy = useRef(false);
  const privateEpoch = useRef(0);
  const authEpoch = useRef(getAuthEpoch());
  const currentAuthEpoch = getAuthEpoch();
  const ownsPrivateState = authEpoch.current === currentAuthEpoch;
  const isFocused = useRef(false);
  const isVisible = useRef(AppState.currentState === "active");
  const [appActive, setAppActive] = useState(
    AppState.currentState === "active",
  );

  const clearSensitiveInputs = useCallback(() => {
    privateEpoch.current += 1;
    setEnrollment(null);
    setTotpCode("");
    setRevealCodes(false);
    setDisableOpen(false);
    setPassword("");
    setDisableCode("");
    setFeedback(null);
  }, []);

  const clearPrivateState = useCallback(() => {
    clearSensitiveInputs();
    recoveryLifecycle.clear();
    syncRecovery();
    setEnrollmentPending(false);
  }, [clearSensitiveInputs, recoveryLifecycle, syncRecovery]);

  useEffect(() => {
    if (authEpoch.current !== currentAuthEpoch) clearPrivateState();
  }, [clearPrivateState, currentAuthEpoch]);

  useWorkspaceBack({
    active: false,
    defaultRole: "back",
    onBack: () => {},
    disabled:
      ownsPrivateState &&
      (privateAction === "confirm" ||
        recovery.confirming ||
        recovery.count > 0),
  });

  useFocusEffect(
    useCallback(() => {
      isFocused.current = true;
      isVisible.current = AppState.currentState === "active";
      recoveryLifecycle.setFocused(true);
      recoveryLifecycle.setActive(isVisible.current);
      syncRecovery();
      setAppActive(isVisible.current);
      void profile.refetch();
      void sessions.refetch();
      return () => {
        isFocused.current = false;
        recoveryLifecycle.setFocused(false);
        clearPrivateState();
      };
    }, [
      clearPrivateState,
      profile.refetch,
      recoveryLifecycle,
      sessions.refetch,
      syncRecovery,
    ]),
  );

  useEffect(() => {
    const subscription = AppState.addEventListener("change", (state) => {
      isVisible.current = state === "active";
      recoveryLifecycle.setActive(isVisible.current);
      syncRecovery();
      setAppActive(isVisible.current);
      // A share chooser can background Android without saving anything.
      // Mask one-time output until foreground; discard it only on leaving
      // this screen or explicit acknowledgment. Clear reusable inputs now.
      if (state === "background") clearSensitiveInputs();
      // Inactive is transient (Face ID, password AutoFill, Control Center):
      // mask codes but keep inputs and in-flight actions.
      else if (state === "inactive") setRevealCodes(false);
      else if (isFocused.current) {
        void profile.refetch();
        void sessions.refetch();
      }
    });
    return () => subscription.remove();
  }, [
    clearSensitiveInputs,
    profile.refetch,
    recoveryLifecycle,
    sessions.refetch,
    syncRecovery,
  ]);

  const currentEpoch = (epoch: number) =>
    epoch === privateEpoch.current &&
    authEpoch.current === getAuthEpoch() &&
    isFocused.current &&
    isVisible.current;
  const refreshProfile = () =>
    queryClient.invalidateQueries({ queryKey: ["account", "profile"] });

  async function runPrivate(
    action: PrivateAction,
    operation: (epoch: number) => Promise<void>,
  ) {
    if (
      privateBusy.current ||
      !isFocused.current ||
      !isVisible.current ||
      authEpoch.current !== getAuthEpoch()
    )
      return;
    privateBusy.current = true;
    const epoch = privateEpoch.current;
    setPrivateAction(action);
    setFeedback(null);
    try {
      await operation(epoch);
    } catch (error) {
      if (currentEpoch(epoch))
        setFeedback({ message: accountError(error), tone: "error" });
    } finally {
      privateBusy.current = false;
      setPrivateAction(null);
    }
  }

  function beginEnrollment() {
    void runPrivate("setup", async (epoch) => {
      const result = await accountApi.setupMfa();
      if (currentEpoch(epoch)) {
        setEnrollment(result);
        setEnrollmentPending(true);
        setTotpCode("");
      }
    });
  }

  function confirmEnrollment() {
    const result = totpCodeSchema.safeParse(totpCode);
    if (!result.success) {
      setFeedback({
        message: result.error.issues[0]?.message ?? "Enter a valid code",
        tone: "error",
      });
      return;
    }
    void runPrivate("confirm", async () => {
      const ticket = recoveryLifecycle.beginConfirmation();
      if (!ticket) return;
      syncRecovery();
      try {
        const confirmation = await accountApi.confirmMfa(result.data);
        if (
          recoveryLifecycle.completeConfirmation(
            ticket,
            confirmation.recovery_codes,
          )
        ) {
          void refreshProfile();
          setEnrollment(null);
          setEnrollmentPending(false);
          setTotpCode("");
          setRevealCodes(false);
          setFeedback(
            isVisible.current
              ? {
                  message: "Two-factor authentication enabled",
                  tone: "success",
                }
              : null,
          );
        }
      } catch (error) {
        if (recoveryLifecycle.finishConfirmation(ticket) && isVisible.current)
          setFeedback({ message: accountError(error), tone: "error" });
      } finally {
        syncRecovery();
      }
    });
  }

  function disableMfa() {
    const epoch = privateEpoch.current;
    const result = disableMfaSchema.safeParse({
      code: disableCode,
      ...(passwordAccount ? { password } : {}),
    });
    if (!result.success) {
      setFeedback({
        message:
          "Enter your authenticator or recovery code and, if required, your current password.",
        tone: "error",
      });
      return;
    }
    Alert.alert(
      "Turn off two-factor authentication?",
      "Your account will no longer require an authenticator code when you sign in with a password.",
      [
        { text: "Cancel", style: "cancel" },
        {
          text: "Turn off",
          style: "destructive",
          onPress: () => {
            if (!currentEpoch(epoch)) return;
            void runPrivate("disable", async (epoch) => {
              await accountApi.disableMfa(result.data, {
                authorityGuard: () => {
                  if (!currentEpoch(epoch))
                    throw Object.assign(new Error("Account action cancelled"), {
                      name: "AbortError",
                    });
                },
              });
              if (currentEpoch(epoch)) {
                void refreshProfile();
                clearPrivateState();
                setFeedback({
                  message: "Two-factor authentication turned off",
                  tone: "success",
                });
              }
            });
          },
        },
      ],
    );
  }

  function copyPrivate(value: string, message: string) {
    void runPrivate("copy", async (epoch) => {
      await Clipboard.setStringAsync(value);
      if (currentEpoch(epoch)) setFeedback({ message, tone: "success" });
    });
  }

  function exportCodes() {
    void runPrivate("share", async (epoch) => {
      const result = await Share.share({
        title: "NyxID recovery codes",
        message: `NyxID recovery codes\n\n${recoveryCodes.join("\n")}`,
      });
      if (currentEpoch(epoch) && result.action === Share.sharedAction)
        setFeedback({
          message:
            "Confirm you saved a private copy before closing. You can export again.",
          tone: "info",
        });
    });
  }

  const passwordAccount =
    !!profile.data &&
    (profile.data.has_password ?? !profile.data.social_provider);
  const mfaEnabled = profile.data?.mfa_enabled === true || recovery.count > 0;
  const privateDisabled = privateAction !== null || !isConnected;

  return (
    <SettingsPage>
      {!isConnected ? (
        <Notice
          message={t("You're offline. Connect to update account settings.")}
        />
      ) : null}
      {feedback ? <Notice {...feedback} /> : null}

      {profile.isPending ? (
        <SkeletonRows
          rows={3}
          icon={false}
          title
          accessibilityLabel={t("Loading your account")}
        />
      ) : (
        <SettingsGroup title={t("Profile")} inset={false}>
          {profile.isError || !profile.data ? (
            <SettingsRow
              label={t("Couldn't load your profile")}
              detail={t("Tap to retry.")}
              tone="action"
              onPress={() => void profile.refetch()}
            />
          ) : (
            [
              <SettingsRow
                key="email"
                label={t("Email")}
                detail={
                  profile.data.email_verified
                    ? t("Verified")
                    : t("Not verified")
                }
                value={profile.data.email}
              />,
              <SettingsRow
                key="name"
                label={t("Display name")}
                value={profile.data.display_name || t("Not set")}
                disabled={!isConnected}
                onPress={() =>
                  navigation.navigate("ProfileEdit", { field: "display_name" })
                }
              />,
              <SettingsRow
                key="avatar"
                label={t("Avatar")}
                value={
                  profile.data.avatar_url ? t("Custom image") : t("Initial")
                }
                disabled={!isConnected}
                onPress={() =>
                  navigation.navigate("ProfileEdit", { field: "avatar_url" })
                }
              />,
            ]
          )}
        </SettingsGroup>
      )}

      {profile.data && ownsPrivateState ? (
        <>
          <SettingsGroup
            title={t("Two-factor authentication")}
            inset={false}
            footer={
              recovery.count > 0
                ? t(
                    "These codes are shown once. Keep a private copy, then choose “I've Saved the Codes”. Continue signing in with your authenticator; recovery-code sign-in is currently unavailable.",
                  )
                : enrollmentPending && !mfaEnabled
                  ? t(
                      "In your authenticator, add a time-based NyxID account with this setup key, then enter its six-digit code.",
                    )
                  : mfaEnabled
                    ? t(
                        "Password sign-in asks for a code from your authenticator.",
                      )
                    : passwordAccount
                      ? t(
                          "Add a six-digit authenticator code to password sign-in.",
                        )
                      : `This account uses ${profile.data.social_provider} sign-in. You can still add an authenticator; turning it off later needs only an authenticator or recovery code.`
            }
          >
            <SettingsRow
              label={t("Two-factor authentication")}
              value={mfaEnabled ? "On" : "Off"}
            />
            {recovery.count > 0 && !appActive ? (
              <SettingsRow
                label={t(
                  "Recovery codes are hidden while the app is inactive.",
                )}
              />
            ) : recovery.count > 0 ? (
              [
                <SettingsRow
                  key="codes"
                  mono
                  selectable={revealCodes}
                  accessibilityHint={
                    revealCodes
                      ? t("Recovery codes")
                      : t("Recovery codes hidden")
                  }
                  label={
                    revealCodes
                      ? recoveryCodes.join("\n")
                      : `${recoveryCodes.length} private codes · hidden`
                  }
                />,
                <SettingsRow
                  key="reveal"
                  tone="action"
                  label={revealCodes ? t("Hide Codes") : t("Reveal Codes")}
                  onPress={() => setRevealCodes(!revealCodes)}
                />,
                <SettingsRow
                  key="copy"
                  tone="action"
                  label={t("Copy Recovery Codes")}
                  disabled={privateDisabled}
                  onPress={() =>
                    copyPrivate(
                      recoveryCodes.join("\n"),
                      "Codes copied. Paste them into a private password manager.",
                    )
                  }
                />,
                <SettingsRow
                  key="export"
                  tone="action"
                  label={
                    privateAction === "share"
                      ? t("Exporting…")
                      : t("Export Recovery Codes")
                  }
                  disabled={privateDisabled}
                  onPress={exportCodes}
                />,
                <SettingsRow
                  key="saved"
                  tone="action"
                  label={t("I've Saved the Codes")}
                  disabled={privateAction !== null}
                  onPress={clearPrivateState}
                />,
              ]
            ) : enrollmentPending && !mfaEnabled ? (
              [
                enrollment && appActive ? (
                  <SettingsRow
                    key="key"
                    label={t("Setup key")}
                    detail={enrollment.secret}
                    selectable
                  />
                ) : (
                  <SettingsRow
                    key="key"
                    label={t(
                      "Enter the code from the NyxID account you just added. If you did not add it, cancel and start again.",
                    )}
                  />
                ),
                enrollment && appActive ? (
                  <SettingsRow
                    key="copy-key"
                    tone="action"
                    label={t("Copy Setup Key")}
                    disabled={privateDisabled}
                    onPress={() =>
                      copyPrivate(
                        enrollment.secret,
                        "Setup key copied. Add it to your authenticator.",
                      )
                    }
                  />
                ) : null,
                enrollment && appActive ? (
                  <SettingsRow
                    key="open"
                    tone="action"
                    label={t("Open Authenticator")}
                    disabled={privateDisabled}
                    onPress={() => {
                      const url = enrollment.qr_code_url;
                      void Linking.openURL(url).catch(() => {
                        if (isFocused.current)
                          setFeedback({
                            message:
                              "No authenticator opened. Copy the setup key and add the account manually.",
                            tone: "info",
                          });
                      });
                    }}
                  />
                ) : null,
                <SettingsTextField
                  key="code"
                  accessibilityLabel={t("Authenticator code")}
                  placeholder={t("Six-digit code")}
                  value={totpCode}
                  onChangeText={setTotpCode}
                  keyboardType="number-pad"
                  textContentType="oneTimeCode"
                  autoComplete="one-time-code"
                  maxLength={6}
                  editable={!privateAction}
                />,
                <SettingsRow
                  key="verify"
                  tone="action"
                  label={
                    privateAction === "confirm"
                      ? t("Verifying…")
                      : t("Verify & Turn On")
                  }
                  disabled={privateDisabled || !/^\d{6}$/.test(totpCode.trim())}
                  onPress={confirmEnrollment}
                />,
                <SettingsRow
                  key="cancel"
                  tone="danger"
                  label={t("Cancel Setup")}
                  disabled={privateAction !== null}
                  onPress={clearPrivateState}
                />,
              ]
            ) : mfaEnabled ? (
              disableOpen ? (
                [
                  <SettingsTextField
                    key="disable-code"
                    accessibilityLabel={t("Authenticator or recovery code")}
                    placeholder={t("Authenticator or recovery code")}
                    value={disableCode}
                    onChangeText={setDisableCode}
                    autoCorrect={false}
                    autoCapitalize="none"
                    maxLength={128}
                    secureTextEntry
                    editable={!privateAction}
                  />,
                  passwordAccount ? (
                    <SettingsTextField
                      key="password"
                      accessibilityLabel={t("Current password")}
                      placeholder={t("Current password")}
                      value={password}
                      onChangeText={setPassword}
                      secureTextEntry
                      textContentType="password"
                      autoComplete="current-password"
                      editable={!privateAction}
                    />
                  ) : null,
                  <SettingsRow
                    key="confirm-off"
                    tone="danger"
                    label={
                      privateAction === "disable"
                        ? t("Turning off…")
                        : t("Turn Off Two-Factor Authentication")
                    }
                    disabled={
                      privateDisabled ||
                      !disableCode.trim() ||
                      (passwordAccount && !password)
                    }
                    onPress={disableMfa}
                  />,
                  <SettingsRow
                    key="cancel-off"
                    tone="action"
                    label={t("Cancel")}
                    disabled={privateAction !== null}
                    onPress={clearPrivateState}
                  />,
                ]
              ) : (
                <SettingsRow
                  tone="danger"
                  label={t("Turn Off Two-Factor Authentication")}
                  disabled={privateDisabled}
                  onPress={() => setDisableOpen(true)}
                />
              )
            ) : (
              <SettingsRow
                tone="action"
                label={
                  privateAction === "setup"
                    ? t("Starting…")
                    : t("Set Up Authenticator")
                }
                disabled={privateDisabled}
                onPress={beginEnrollment}
              />
            )}
          </SettingsGroup>
          <AccountSecurityControls
            key={`security:${profile.data.id}`}
            profile={profile.data}
          />
          <AccountAuthorizations
            key={`access:${profile.data.id}`}
            accountId={profile.data.id}
          />
          <AccountOrganizations
            key={`orgs:${profile.data.id}`}
            accountId={profile.data.id}
            email={profile.data.email}
          />
        </>
      ) : null}

      {profile.isPending ? (
        <SkeletonRows
          rows={3}
          icon={false}
          title
          accessibilityLabel={t("Loading security settings")}
        />
      ) : null}
      {sessions.isPending ? (
        <SkeletonRows
          rows={2}
          icon={false}
          title
          accessibilityLabel={t("Loading sessions")}
        />
      ) : (
        <SettingsGroup
          title={t("Active sessions")}
          inset={false}
          footer={t(
            "Where your account is signed in. Tap a session for details or to sign it out.",
          )}
        >
          {sessions.isError ? (
            <SettingsRow
              label={t("Couldn't load active sessions")}
              detail={t("Tap to retry.")}
              tone="action"
              onPress={() => void sessions.refetch()}
            />
          ) : sessions.data?.length === 0 ? (
            <SettingsRow label={t("No active sessions found.")} />
          ) : (
            sessions.data?.map((session) => (
              <SettingsRow
                key={session.id}
                label={sessionDeviceLabel(session.user_agent)}
                detail={`Signed in ${formatSessionDate(session.created_at)}`}
                value={session.is_current ? "This session" : undefined}
                onPress={() =>
                  navigation.navigate("SessionDetail", {
                    sessionId: session.id,
                  })
                }
              />
            ))
          )}
        </SettingsGroup>
      )}
    </SettingsPage>
  );
}
