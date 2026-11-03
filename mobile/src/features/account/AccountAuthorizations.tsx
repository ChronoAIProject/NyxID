import { useRef, useState } from "react";
import { Alert } from "react-native";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Notice } from "../../components/workspace/WorkspaceUI";
import {
  SettingsGroup,
  SettingsRow,
} from "../../components/settings/SettingsList";
import { accountApi } from "../../lib/api/accountApi";
import { resolveErrorMessage } from "../../lib/api/errorMessages";
import { useNetworkStatus } from "../../hooks/useNetworkStatus";
import {
  useAccountSecurityRefresh,
  type AccountSecurityTicket,
} from "./useAccountSecurityRefresh";

export function AccountAuthorizations({ accountId }: { accountId: string }) {
  const client = useQueryClient();
  const { isConnected } = useNetworkStatus();
  const key = ["account", "authorizations", accountId];
  const apps = useQuery({
    queryKey: [...key, "apps"],
    queryFn: ({ signal }) => accountApi.consents(accountId, signal),
  });
  const bindings = useQuery({
    queryKey: [...key, "bindings"],
    queryFn: ({ signal }) => accountApi.brokerBindings(accountId, signal),
  });
  const [busy, setBusy] = useState(false);
  const locked = useRef(false);
  const [feedback, setFeedback] = useState<{
    message: string;
    tone: "error" | "success";
  } | null>(null);
  const refresh = () => {
    void apps.refetch();
    void bindings.refetch();
  };
  const current = useAccountSecurityRefresh(refresh, () => setFeedback(null));
  async function revoke(
    kind: "app" | "binding",
    id: string,
    ticket: AccountSecurityTicket,
  ) {
    if (locked.current || !ticket.isCurrent()) return;
    locked.current = true;
    setBusy(true);
    setFeedback(null);
    try {
      const guard = { authorityGuard: ticket.assertCurrent };
      if (kind === "app") await accountApi.revokeConsent(accountId, id, guard);
      else await accountApi.revokeBrokerBinding(accountId, id, guard);
      if (!ticket.isCurrent()) return;
      // Reset rather than remove one local row: the server decides the resulting authority.
      const listed = await client
        .resetQueries({ queryKey: key }, { throwOnError: true })
        .then(
          () => true,
          () => false,
        );
      if (ticket.isCurrent())
        setFeedback({
          message: listed
            ? "Access revoked. Lists refreshed from NyxID."
            : "Access revoked. Lists could not reload yet; retry to confirm.",
          tone: "success",
        });
    } catch (error) {
      if (ticket.isCurrent())
        setFeedback({ message: resolveErrorMessage(error), tone: "error" });
    } finally {
      locked.current = false;
      setBusy(false);
    }
  }
  function confirm(kind: "app" | "binding", id: string, label: string) {
    const ticket = current.capture();
    Alert.alert(
      `Revoke ${label}?`,
      kind === "app"
        ? "This application will need consent again. Related Machine permissions are revoked with this consent."
        : "This app will need authorization again before it can act on your behalf.",
      [
        { text: "Cancel", style: "cancel" },
        {
          text: "Revoke",
          style: "destructive",
          onPress: () => void revoke(kind, id, ticket),
        },
      ],
    );
  }
  const disabled =
    !isConnected || busy || apps.isFetching || bindings.isFetching;
  const services = (app: NonNullable<typeof apps.data>[number]) =>
    app.legacy_unrestricted
      ? "Legacy unrestricted service access"
      : app.allow_all_services
        ? "All services"
        : app.allowed_services
            .map(
              (service) =>
                `${service.label}${service.deleted ? " (deleted)" : ""}`,
            )
            .join(", ") || "No services";
  return (
    <>
      {feedback ? <Notice {...feedback} /> : null}
      <SettingsGroup
        title="Authorized apps"
        inset={false}
        footer="Apps you signed in to with NyxID. Tap one to revoke it. These are separate from approval grants in Activity."
      >
        {apps.isPending ? (
          <SettingsRow label="Loading authorized apps…" />
        ) : apps.isError ? (
          <SettingsRow
            label="Couldn't load authorized apps"
            detail="Tap to retry."
            tone="action"
            onPress={refresh}
          />
        ) : apps.data?.length === 0 ? (
          <SettingsRow label="No authorized apps" />
        ) : (
          apps.data?.map((app) => (
            <SettingsRow
              key={app.client_id}
              label={app.client_name}
              detail={`${services(app)} · ${app.scopes || "No scopes"}\nGranted ${app.granted_at} · Expires ${app.expires_at ?? "never"}`}
              value="Revoke"
              disabled={disabled}
              accessibilityHint={`Revoke ${app.client_name}`}
              onPress={() => confirm("app", app.client_id, app.client_name)}
            />
          ))
        )}
      </SettingsGroup>
      <SettingsGroup
        title="Broker authorizations"
        inset={false}
        footer="Apps acting for you through NyxID's broker."
      >
        {bindings.isPending ? (
          <SettingsRow label="Loading broker authorizations…" />
        ) : bindings.isError ? (
          <SettingsRow
            label="Couldn't load broker authorizations"
            detail="Tap to retry."
            tone="action"
            onPress={refresh}
          />
        ) : bindings.data?.length === 0 ? (
          <SettingsRow label="No broker authorizations" />
        ) : (
          bindings.data?.map((binding) => {
            const name = binding.client_name ?? binding.client_id;
            return (
              <SettingsRow
                key={binding.binding_hash}
                label={name}
                detail={`${
                  binding.external_subject
                    ? `${binding.external_subject.platform} · ${binding.external_subject.external_user_id}\n`
                    : ""
                }${binding.scopes.join(" ")}\nCreated ${binding.created_at} · Last used ${binding.last_used_at ?? "never"}`}
                value="Revoke"
                disabled={disabled}
                accessibilityHint={`Revoke ${binding.client_name ?? "authorization"}`}
                onPress={() =>
                  confirm(
                    "binding",
                    binding.binding_hash,
                    binding.client_name ?? "authorization",
                  )
                }
              />
            );
          })
        )}
        <SettingsRow
          tone="action"
          label={
            apps.isFetching || bindings.isFetching
              ? "Refreshing…"
              : "Refresh Authorizations"
          }
          disabled={busy || !isConnected}
          onPress={refresh}
        />
      </SettingsGroup>
    </>
  );
}
