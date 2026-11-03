import { useRef, useState } from "react";
import { Alert, Switch } from "react-native";
import { useQueryClient } from "@tanstack/react-query";
import {
  ActionButton,
  Card,
  Field,
  Label,
  Notice,
} from "../../components/workspace/WorkspaceUI";
import { accountApi } from "../../lib/api/accountApi";
import { changePasswordSchema } from "../../lib/api/accountSecurityTransport";
import type { AccountProfile } from "../../lib/api/types";
import { resolveErrorMessage } from "../../lib/api/errorMessages";
import { useNetworkStatus } from "../../hooks/useNetworkStatus";
import { useMfaStepUp } from "../auth/useMfaStepUp";
import { useAccountSecurityRefresh } from "./useAccountSecurityRefresh";

export function AccountSecurityControls({
  profile,
}: {
  profile: AccountProfile;
}) {
  const client = useQueryClient();
  const { isConnected } = useNetworkStatus();
  const [currentPassword, setCurrentPassword] = useState("");
  const [newPassword, setNewPassword] = useState("");
  const [confirmation, setConfirmation] = useState("");
  const [busy, setBusy] = useState(false);
  const locked = useRef(false);
  const [message, setMessage] = useState<{
    message: string;
    tone: "success" | "error";
  } | null>(null);
  const stepUp = useMfaStepUp(`account-security:${profile.id}`);
  const clear = () => {
    setCurrentPassword("");
    setNewPassword("");
    setConfirmation("");
    setMessage(null);
  };
  const refresh = async () => {
    await client.invalidateQueries({ queryKey: ["account", "profile"] });
  };
  const current = useAccountSecurityRefresh(() => {}, clear);
  async function changePassword() {
    if (locked.current || !current()) return;
    const parsed = changePasswordSchema.safeParse({
      current_password: currentPassword,
      new_password: newPassword,
      confirmation,
    });
    if (!parsed.success) {
      setMessage({
        message: parsed.error.issues[0]?.message ?? "Check password fields",
        tone: "error",
      });
      return;
    }
    locked.current = true;
    setBusy(true);
    setMessage(null);
    try {
      await accountApi.changePassword(profile.id, parsed.data);
      if (current()) {
        clear();
        await refresh();
        if (current())
          setMessage({
            message: "Password changed successfully.",
            tone: "success",
          });
      }
    } catch (error) {
      if (current())
        setMessage({ message: resolveErrorMessage(error), tone: "error" });
    } finally {
      locked.current = false;
      setBusy(false);
    }
  }
  async function savePolicy(enabled: boolean) {
    if (locked.current || !current()) return;
    locked.current = true;
    setBusy(true);
    setMessage(null);
    try {
      await accountApi.saveSecurityPreferences(profile.id, {
        skip_mfa_for_approved_sign_ins: enabled,
      });
      if (current()) {
        await refresh();
        if (current())
          setMessage({ message: "Sign-in policy saved", tone: "success" });
      }
    } catch (error) {
      if (current() && !(await stepUp.handle(error, () => savePolicy(enabled))))
        setMessage({ message: resolveErrorMessage(error), tone: "error" });
    } finally {
      locked.current = false;
      setBusy(false);
    }
  }
  const policy = profile.profile_config?.security;
  const disabled = busy || stepUp.busy || !isConnected;
  return (
    <Card>
      <Label accessibilityRole="header">Password & approved sign-ins</Label>
      {message ? <Notice {...message} /> : null}
      {profile.has_password === true ? (
        <>
          <Field
            label="Current password for password change"
            value={currentPassword}
            onChangeText={setCurrentPassword}
            secureTextEntry
            autoComplete="current-password"
            editable={!disabled}
          />
          <Field
            label="New password"
            value={newPassword}
            onChangeText={setNewPassword}
            secureTextEntry
            autoComplete="new-password"
            editable={!disabled}
          />
          <Field
            label="Confirm new password"
            value={confirmation}
            onChangeText={setConfirmation}
            secureTextEntry
            autoComplete="new-password"
            editable={!disabled}
          />
          <ActionButton
            label="Change password"
            onPress={() => void changePassword()}
            loading={busy}
            disabled={
              disabled || !currentPassword || !newPassword || !confirmation
            }
          />
        </>
      ) : (
        <Label muted>
          {profile.has_password === false
            ? "This account uses social sign-in and has no password to change."
            : "This server does not report password availability. Update it before changing passwords in the app."}
        </Label>
      )}
      <Label>Skip MFA for NyxID app sign-ins</Label>
      <Label muted>
        Approve QR or typed sign-in codes without another authenticator code.
        Only approve codes shown on your own screen.{" "}
        {profile.mfa_enabled
          ? ""
          : "Takes effect when two-factor authentication is on."}
      </Label>
      {!policy ? (
        <Label muted>
          This server does not support changing this policy yet.
        </Label>
      ) : null}
      <Switch
        accessibilityLabel="Skip MFA for NyxID app sign-ins"
        value={policy?.skip_mfa_for_approved_sign_ins ?? false}
        disabled={!policy || disabled}
        onValueChange={(enabled) => {
          if (!enabled) {
            void savePolicy(false);
            return;
          }
          Alert.alert(
            "Skip MFA for app-approved sign-ins?",
            "Anyone with access to your signed-in NyxID app or browser could approve sign-ins without another code.",
            [
              { text: "Cancel", style: "cancel" },
              { text: "Enable", onPress: () => void savePolicy(true) },
            ],
          );
        }}
      />
      {stepUp.prompt}
    </Card>
  );
}
