import { useCallback, useEffect, useRef, useState } from "react";
import { useFocusEffect } from "@react-navigation/native";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Alert, AppState, Linking, Share, View } from "react-native";
import * as Clipboard from "expo-clipboard";
import { useWorkspaceBack } from "../../app/WorkspaceHeader";
import {
  ActionButton,
  Card,
  Field,
  Label,
  Notice,
  WorkspacePage,
} from "../../components/workspace/WorkspaceUI";
import { useNetworkStatus } from "../../hooks/useNetworkStatus";
import { accountApi } from "../../lib/api/accountApi";
import {
  totpCodeSchema,
  updateProfileSchema,
  type AccountSession,
  type MfaEnrollment,
} from "../../lib/api/accountSchema";
import { isApiError } from "../../lib/api/ApiError";
import { resolveErrorMessage } from "../../lib/api/errorMessages";
import { getAuthEpoch } from "../../lib/auth/authEpoch";
import { useTheme } from "../../theme/ThemeContext";
import { useAuthSession } from "../auth/AuthSessionContext";
import { createMfaRecoveryLifecycle } from "./mfaRecoveryLifecycle";

type PrivateAction = "setup" | "confirm" | "disable" | "copy" | "share";
type Feedback = { message: string; tone: "error" | "info" | "success" };

function accountError(error: unknown): string {
  if (isApiError(error) && error.errorCode === 2000)
    return "The password or authenticator code is incorrect. Please try again.";
  return resolveErrorMessage(error);
}

function formatDate(value: string): string {
  const date = new Date(value);
  return Number.isFinite(date.getTime())
    ? date.toLocaleString()
    : "Unavailable";
}

export function AccountSecurityScreen() {
  const { colors } = useTheme();
  const { signOut } = useAuthSession();
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
  const [displayName, setDisplayName] = useState("");
  const [avatarUrl, setAvatarUrl] = useState("");
  const profileInitialized = useRef(false);
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

  useEffect(() => {
    if (!profile.data || profileInitialized.current) return;
    profileInitialized.current = true;
    setDisplayName(profile.data.display_name ?? "");
    setAvatarUrl(profile.data.avatar_url ?? "");
  }, [profile.data]);

  const clearSensitiveInputs = useCallback(() => {
    privateEpoch.current += 1;
    setEnrollment(null);
    setTotpCode("");
    setRevealCodes(false);
    setDisableOpen(false);
    setPassword("");
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

  const updateProfile = useMutation({
    mutationFn: accountApi.updateProfile,
    onSuccess: (result) => {
      setDisplayName(result.display_name ?? "");
      setAvatarUrl(result.avatar_url ?? "");
      void refreshProfile();
      setFeedback({ message: "Profile updated", tone: "success" });
    },
    onError: (error) =>
      setFeedback({ message: accountError(error), tone: "error" }),
  });

  const revokeSession = useMutation({
    mutationFn: (session: AccountSession) =>
      accountApi.revokeSession(session.id),
    onSuccess: async (_, session) => {
      if (session.is_current) {
        await signOut();
        queryClient.clear();
        return;
      }
      void queryClient.invalidateQueries({ queryKey: ["account", "sessions"] });
      setFeedback({ message: "Session signed out", tone: "success" });
    },
    onError: (error) =>
      setFeedback({ message: accountError(error), tone: "error" }),
  });

  function saveProfile() {
    if (!profile.data) return;
    const changes = {
      ...(displayName.trim() !== (profile.data.display_name ?? "")
        ? { display_name: displayName }
        : {}),
      ...(avatarUrl.trim() !== (profile.data.avatar_url ?? "")
        ? { avatar_url: avatarUrl }
        : {}),
    };
    const result = updateProfileSchema.safeParse(changes);
    if (!result.success) {
      setFeedback({
        message:
          result.error.issues[0]?.message ?? "Check your profile details",
        tone: "error",
      });
      return;
    }
    setFeedback(null);
    updateProfile.mutate(result.data);
  }

  function confirmSessionRevocation(session: AccountSession) {
    Alert.alert(
      session.is_current
        ? "Sign out of this session?"
        : "Sign out this session?",
      session.is_current
        ? "You will return to the sign-in screen on this device."
        : "This session will lose access to your account. You can sign in again later.",
      [
        { text: "Cancel", style: "cancel" },
        {
          text: "Sign out",
          style: "destructive",
          onPress: () => revokeSession.mutate(session),
        },
      ],
    );
  }

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
    if (!password) return;
    Alert.alert(
      "Turn off two-factor authentication?",
      "Your account will no longer require an authenticator code when you sign in with a password.",
      [
        { text: "Cancel", style: "cancel" },
        {
          text: "Turn off",
          style: "destructive",
          onPress: () => {
            void runPrivate("disable", async (epoch) => {
              await accountApi.disableMfa(password);
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

  const profileChanged =
    !!profile.data &&
    (displayName.trim() !== (profile.data.display_name ?? "") ||
      avatarUrl.trim() !== (profile.data.avatar_url ?? ""));
  const passwordAccount = !!profile.data && !profile.data.social_provider;
  const mfaEnabled = profile.data?.mfa_enabled === true || recovery.count > 0;
  const privateDisabled = privateAction !== null || !isConnected;

  return (
    <WorkspacePage
      title="Account & security"
      subtitle="Manage your identity and protect access to NyxID."
    >
      {!isConnected ? (
        <Notice message="You're offline. Connect to update account settings." />
      ) : null}
      {feedback ? <Notice {...feedback} /> : null}

      <Card>
        <Label accessibilityRole="header">Profile</Label>
        {profile.isPending ? (
          <Label muted>Loading your account…</Label>
        ) : profile.isError || !profile.data ? (
          <>
            <Notice tone="error" message="Couldn't load your profile." />
            <ActionButton
              label="Retry profile"
              variant="secondary"
              onPress={() => {
                void profile.refetch();
              }}
            />
          </>
        ) : (
          <>
            <Label muted>{profile.data.email}</Label>
            <Label muted>
              {profile.data.email_verified
                ? "Email verified"
                : "Email not verified"}
            </Label>
            <Field
              label="Display name"
              value={displayName}
              onChangeText={setDisplayName}
              autoCapitalize="words"
              editable={!updateProfile.isPending}
            />
            <Field
              label="Avatar URL"
              value={avatarUrl}
              onChangeText={setAvatarUrl}
              keyboardType="url"
              autoCorrect={false}
              editable={!updateProfile.isPending}
              placeholder="https://example.com/avatar.png"
            />
            <ActionButton
              label="Save profile"
              onPress={saveProfile}
              loading={updateProfile.isPending}
              disabled={!isConnected || !profileChanged}
            />
            {profileChanged ? (
              <ActionButton
                label="Discard changes"
                variant="secondary"
                disabled={updateProfile.isPending}
                onPress={() => {
                  setDisplayName(profile.data?.display_name ?? "");
                  setAvatarUrl(profile.data?.avatar_url ?? "");
                }}
              />
            ) : null}
          </>
        )}
      </Card>

      <Card>
        <Label accessibilityRole="header">Two-factor authentication</Label>
        {profile.isPending ? (
          <Label muted>Loading security settings…</Label>
        ) : !profile.data || !ownsPrivateState ? (
          <Label muted>Load your profile to manage security.</Label>
        ) : (
          <>
            <Label muted>
              {mfaEnabled
                ? "Authenticator protection is on."
                : "Add a six-digit authenticator code to password sign-in."}
            </Label>
            {recovery.count > 0 && !appActive ? (
              <Label muted>
                Recovery codes are hidden while the app is inactive.
              </Label>
            ) : recovery.count > 0 ? (
              <>
                <Label>Save your recovery codes</Label>
                <Label muted>
                  These codes are shown once. Keep a private copy. Continue
                  signing in with your authenticator; recovery-code sign-in is
                  currently unavailable.
                </Label>
                <Label muted>
                  Save a private copy, then choose “I've saved the codes” to
                  leave this screen.
                </Label>
                <View
                  accessibilityLabel={
                    revealCodes ? "Recovery codes" : "Recovery codes hidden"
                  }
                >
                  <Label mono selectable={revealCodes}>
                    {revealCodes
                      ? recoveryCodes.join("\n")
                      : `${recoveryCodes.length} private codes · hidden`}
                  </Label>
                </View>
                <ActionButton
                  label={revealCodes ? "Hide codes" : "Reveal codes"}
                  variant="secondary"
                  onPress={() => setRevealCodes(!revealCodes)}
                />
                <ActionButton
                  label="Copy recovery codes"
                  variant="secondary"
                  disabled={privateDisabled}
                  onPress={() =>
                    copyPrivate(
                      recoveryCodes.join("\n"),
                      "Codes copied. Paste them into a private password manager.",
                    )
                  }
                />
                <ActionButton
                  label="Export recovery codes"
                  variant="secondary"
                  disabled={privateDisabled}
                  loading={privateAction === "share"}
                  onPress={exportCodes}
                />
                <ActionButton
                  label="I've saved the codes"
                  disabled={privateAction !== null}
                  onPress={clearPrivateState}
                />
              </>
            ) : enrollmentPending && !mfaEnabled ? (
              <>
                {enrollment && appActive ? (
                  <>
                    <Label muted>
                      In your authenticator, add a time-based NyxID account
                      using this setup key.
                    </Label>
                    <Label mono selectable>
                      {enrollment.secret}
                    </Label>
                    <ActionButton
                      label="Copy setup key"
                      variant="secondary"
                      disabled={privateDisabled}
                      onPress={() =>
                        copyPrivate(
                          enrollment.secret,
                          "Setup key copied. Add it to your authenticator.",
                        )
                      }
                    />
                    <ActionButton
                      label="Open authenticator"
                      variant="secondary"
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
                  </>
                ) : (
                  <Label muted>
                    Enter the code from the NyxID account you just added to your
                    authenticator. If you did not add it, cancel and start setup
                    again.
                  </Label>
                )}
                <Field
                  label="Authenticator code"
                  value={totpCode}
                  onChangeText={setTotpCode}
                  keyboardType="number-pad"
                  textContentType="oneTimeCode"
                  autoComplete="one-time-code"
                  maxLength={6}
                  editable={!privateAction}
                />
                <ActionButton
                  label="Verify & enable"
                  loading={privateAction === "confirm"}
                  disabled={privateDisabled || !/^\d{6}$/.test(totpCode.trim())}
                  onPress={confirmEnrollment}
                />
                <ActionButton
                  label="Cancel setup"
                  variant="secondary"
                  disabled={privateAction !== null}
                  onPress={clearPrivateState}
                />
              </>
            ) : mfaEnabled ? (
              disableOpen ? (
                <>
                  <Field
                    label="Current password"
                    value={password}
                    onChangeText={setPassword}
                    secureTextEntry
                    textContentType="password"
                    autoComplete="current-password"
                    editable={!privateAction}
                  />
                  <ActionButton
                    label="Turn off two-factor authentication"
                    variant="danger"
                    loading={privateAction === "disable"}
                    disabled={privateDisabled || !password}
                    onPress={disableMfa}
                  />
                  <ActionButton
                    label="Cancel"
                    variant="secondary"
                    disabled={privateAction !== null}
                    onPress={clearPrivateState}
                  />
                </>
              ) : passwordAccount ? (
                <ActionButton
                  label="Turn off two-factor authentication"
                  variant="secondary"
                  disabled={privateDisabled}
                  onPress={() => setDisableOpen(true)}
                />
              ) : (
                <Label muted>
                  Turning off two-factor authentication requires an account
                  password.
                </Label>
              )
            ) : (
              <>
                {!passwordAccount ? (
                  <Label muted>
                    This account uses {profile.data.social_provider} sign-in.
                    Turning off two-factor authentication later requires an
                    account password.
                  </Label>
                ) : null}
                <ActionButton
                  label="Set up authenticator"
                  loading={privateAction === "setup"}
                  disabled={privateDisabled}
                  onPress={beginEnrollment}
                />
              </>
            )}
          </>
        )}
      </Card>

      <Card>
        <Label accessibilityRole="header">Active sessions</Label>
        <Label muted>
          Review where your account is signed in. Network details come from
          NyxID's session records.
        </Label>
        {sessions.isPending ? (
          <Label muted>Loading sessions…</Label>
        ) : sessions.isError ? (
          <>
            <Notice message="Couldn't load active sessions." tone="error" />
            <ActionButton
              label="Retry sessions"
              variant="secondary"
              onPress={() => {
                void sessions.refetch();
              }}
            />
          </>
        ) : sessions.data?.length === 0 ? (
          <Label muted>No active sessions found.</Label>
        ) : (
          sessions.data?.map((session) => (
            <View
              key={session.id}
              style={{
                gap: 8,
                paddingVertical: 12,
                borderTopWidth: 1,
                borderTopColor: colors.border,
              }}
            >
              <Label>
                {session.is_current ? "This session" : "Signed-in session"}
              </Label>
              <Label muted selectable>
                {session.user_agent || "Device details unavailable"}
              </Label>
              <Label muted>
                Network: {session.ip_address || "Unavailable"}
              </Label>
              <Label muted>Signed in {formatDate(session.created_at)}</Label>
              <Label muted>Expires {formatDate(session.expires_at)}</Label>
              <ActionButton
                label={
                  session.is_current
                    ? "Sign out on this device"
                    : "Sign out session"
                }
                variant="danger"
                disabled={!isConnected || revokeSession.isPending}
                loading={
                  revokeSession.isPending &&
                  revokeSession.variables?.id === session.id
                }
                onPress={() => confirmSessionRevocation(session)}
              />
            </View>
          ))
        )}
        <ActionButton
          label="Refresh sessions"
          variant="secondary"
          disabled={
            !isConnected || sessions.isFetching || revokeSession.isPending
          }
          onPress={() => {
            void sessions.refetch();
          }}
        />
      </Card>
    </WorkspacePage>
  );
}
