import { useRef, useState } from "react";
import { Alert, Switch } from "react-native";
import { useQueryClient } from "@tanstack/react-query";
import { Notice } from "../../components/workspace/WorkspaceUI";
import {
  SettingsGroup,
  SettingsRow,
} from "../../components/settings/SettingsList";
import { useTheme } from "../../theme/ThemeContext";
import { accountApi } from "../../lib/api/accountApi";
import type { AccountProfile } from "../../lib/api/types";
import { resolveErrorMessage } from "../../lib/api/errorMessages";
import { useNetworkStatus } from "../../hooks/useNetworkStatus";
import { useMfaStepUp } from "../auth/useMfaStepUp";
import {
  useAccountSecurityRefresh,
  type AccountSecurityTicket,
} from "./useAccountSecurityRefresh";

export function AccountSecurityControls({
  profile,
}: {
  profile: AccountProfile;
}) {
  const client = useQueryClient();
  const { colors } = useTheme();
  const { isConnected } = useNetworkStatus();
  const [busy, setBusy] = useState(false);
  const locked = useRef(false);
  const [message, setMessage] = useState<{
    message: string;
    tone: "success" | "error";
  } | null>(null);
  const stepUp = useMfaStepUp(`account-security:${profile.id}`);
  const clear = () => {
    setMessage(null);
    stepUp.reset();
  };
  const refresh = async () => {
    await client.invalidateQueries({ queryKey: ["account", "profile"] });
  };
  const current = useAccountSecurityRefresh(() => {}, clear);
  async function savePolicy(
    enabled: boolean,
    ticket: AccountSecurityTicket = current.capture(),
  ) {
    if (locked.current || !ticket.isCurrent()) return;
    locked.current = true;
    setBusy(true);
    setMessage(null);
    try {
      await accountApi.saveSecurityPreferences(
        profile.id,
        { skip_mfa_for_approved_sign_ins: enabled },
        { authorityGuard: ticket.assertCurrent },
      );
      if (ticket.isCurrent()) {
        await refresh();
        if (ticket.isCurrent())
          setMessage({ message: "Sign-in policy saved", tone: "success" });
      }
    } catch (error) {
      if (
        ticket.isCurrent() &&
        !(await stepUp.handle(error, () => savePolicy(enabled, ticket))) &&
        ticket.isCurrent()
      )
        setMessage({ message: resolveErrorMessage(error), tone: "error" });
    } finally {
      locked.current = false;
      setBusy(false);
    }
  }
  const policy = profile.profile_config?.security;
  const disabled = busy || stepUp.busy || !isConnected;
  return (
    <>
      {message ? <Notice {...message} /> : null}
      <SettingsGroup
        title="Approved sign-ins"
        inset={false}
        footer={`Approve QR or typed sign-in codes without another authenticator code. Only approve codes shown on your own screen.${
          profile.mfa_enabled
            ? ""
            : " Takes effect when two-factor authentication is on."
        }${policy ? "" : " This server does not support changing this policy yet."}`}
      >
        <SettingsRow
          label="Skip MFA for NyxID app sign-ins"
          accessory={
            <Switch
              accessibilityLabel="Skip MFA for NyxID app sign-ins"
              value={policy?.skip_mfa_for_approved_sign_ins ?? false}
              disabled={!policy || disabled}
              trackColor={{ false: colors.borderSoft, true: colors.success }}
              onValueChange={(enabled) => {
                const ticket = current.capture();
                if (!enabled) {
                  void savePolicy(false, ticket);
                  return;
                }
                Alert.alert(
                  "Skip MFA for app-approved sign-ins?",
                  "Anyone with access to your signed-in NyxID app or browser could approve sign-ins without another code.",
                  [
                    { text: "Cancel", style: "cancel" },
                    {
                      text: "Enable",
                      onPress: () => void savePolicy(true, ticket),
                    },
                  ],
                );
              }}
            />
          }
        />
      </SettingsGroup>
      {stepUp.prompt}
    </>
  );
}
