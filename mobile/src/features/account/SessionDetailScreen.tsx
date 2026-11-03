import { Alert } from "react-native";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import type { SheetScreenProps } from "../../app/navigationTypes";
import {
  SettingsGroup,
  SettingsPage,
  SettingsRow,
} from "../../components/settings/SettingsList";
import { Label, Notice } from "../../components/workspace/WorkspaceUI";
import { useNetworkStatus } from "../../hooks/useNetworkStatus";
import { accountApi } from "../../lib/api/accountApi";
import { resolveErrorMessage } from "../../lib/api/errorMessages";
import { useAuthSession } from "../auth/AuthSessionContext";
import { sessionDeviceLabel } from "./sessionDevice";

export function formatSessionDate(value: string): string {
  const date = new Date(value);
  return Number.isFinite(date.getTime())
    ? date.toLocaleString()
    : "Unavailable";
}

/** Settings › Account & security › one signed-in session and its sign-out. */
export function SessionDetailScreen({
  navigation,
  route,
}: SheetScreenProps<"SessionDetail">) {
  const { isConnected } = useNetworkStatus();
  const { signOut } = useAuthSession();
  const client = useQueryClient();
  const sessions = useQuery({
    queryKey: ["account", "sessions"],
    queryFn: ({ signal }) => accountApi.listSessions(signal),
  });
  const session = sessions.data?.find((row) => row.id === route.params.sessionId);
  const revoke = useMutation({
    mutationFn: (id: string) => accountApi.revokeSession(id),
    onSuccess: async () => {
      if (session?.is_current) {
        await signOut();
        client.clear();
        return;
      }
      await client.invalidateQueries({ queryKey: ["account", "sessions"] });
      navigation.goBack();
    },
  });
  if (!session)
    return (
      <SettingsPage>
        {sessions.isPending ? (
          <Label muted>Loading session…</Label>
        ) : (
          <Notice message="This session has already ended." />
        )}
      </SettingsPage>
    );
  const current = session.is_current;
  return (
    <SettingsPage>
      {revoke.error ? (
        <Notice tone="error" message={resolveErrorMessage(revoke.error)} />
      ) : null}
      <SettingsGroup
        inset={false}
        footer="Network details come from NyxID's session records."
      >
        <SettingsRow
          label="Device"
          value={sessionDeviceLabel(session.user_agent)}
        />
        {current ? <SettingsRow label="Status" value="This session" /> : null}
        <SettingsRow
          label="Signed in"
          value={formatSessionDate(session.created_at)}
        />
        <SettingsRow
          label="Expires"
          value={formatSessionDate(session.expires_at)}
        />
        <SettingsRow label="Network" value={session.ip_address || "Unavailable"} />
      </SettingsGroup>
      <SettingsGroup inset={false} title="Client">
        <SettingsRow
          label={session.user_agent || "Device details unavailable"}
        />
      </SettingsGroup>
      <SettingsGroup inset={false}>
        <SettingsRow
          centered
          destructive
          label={
            revoke.isPending
              ? "Signing out…"
              : current
                ? "Sign Out on This Device"
                : "Sign Out This Session"
          }
          disabled={!isConnected || revoke.isPending}
          onPress={() =>
            Alert.alert(
              current ? "Sign out of this session?" : "Sign out this session?",
              current
                ? "You will return to the sign-in screen on this device."
                : "This session will lose access to your account. You can sign in again later.",
              [
                { text: "Cancel", style: "cancel" },
                {
                  text: "Sign out",
                  style: "destructive",
                  onPress: () => revoke.mutate(session.id),
                },
              ],
            )
          }
        />
      </SettingsGroup>
    </SettingsPage>
  );
}
