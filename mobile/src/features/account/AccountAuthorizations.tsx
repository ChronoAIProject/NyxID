import { useRef, useState } from "react";
import { Alert } from "react-native";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import {
  ActionButton,
  Card,
  Label,
  Notice,
  Section,
} from "../../components/workspace/WorkspaceUI";
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
  return (
    <Section title="Access & authorizations">
      <Label muted>
        Personal account authorizations. These are separate from approval grants
        in Activity.
      </Label>
      {feedback ? <Notice {...feedback} /> : null}
      <Label>Authorized OAuth apps</Label>
      {apps.isPending ? (
        <Label muted>Loading authorized apps…</Label>
      ) : apps.isError ? (
        <Notice
          tone="error"
          message="Could not load authorized apps. Retry to confirm current access."
        />
      ) : apps.data?.length === 0 ? (
        <Label muted>No authorized apps.</Label>
      ) : (
        apps.data?.map((app) => (
          <Card key={app.client_id}>
            <Label>{app.client_name}</Label>
            <Label muted>{app.scopes || "No scopes"}</Label>
            <Label muted>
              {app.legacy_unrestricted
                ? "Legacy unrestricted service access"
                : app.allow_all_services
                  ? "All services"
                  : app.allowed_services
                      .map(
                        (service) =>
                          `${service.label}${service.deleted ? " (deleted)" : ""}`,
                      )
                      .join(", ") || "No services"}
            </Label>
            <Label muted>
              Granted {app.granted_at} · Expires {app.expires_at ?? "never"}
            </Label>
            <ActionButton
              label={`Revoke ${app.client_name}`}
              variant="danger"
              disabled={disabled}
              onPress={() => confirm("app", app.client_id, app.client_name)}
            />
          </Card>
        ))
      )}
      <Label>Broker authorizations</Label>
      {bindings.isPending ? (
        <Label muted>Loading broker authorizations…</Label>
      ) : bindings.isError ? (
        <Notice
          tone="error"
          message="Could not load broker authorizations. Retry to confirm current access."
        />
      ) : bindings.data?.length === 0 ? (
        <Label muted>No broker authorizations.</Label>
      ) : (
        bindings.data?.map((binding) => (
          <Card key={binding.binding_hash}>
            <Label>{binding.client_name ?? binding.client_id}</Label>
            {binding.external_subject ? (
              <Label muted>
                {binding.external_subject.platform} ·{" "}
                {binding.external_subject.external_user_id}
              </Label>
            ) : null}
            <Label muted>{binding.scopes.join(" ")}</Label>
            <Label muted>
              Created {binding.created_at} · Last used{" "}
              {binding.last_used_at ?? "never"}
            </Label>
            <ActionButton
              label={`Revoke ${binding.client_name ?? "authorization"}`}
              variant="danger"
              disabled={disabled}
              onPress={() =>
                confirm(
                  "binding",
                  binding.binding_hash,
                  binding.client_name ?? "authorization",
                )
              }
            />
          </Card>
        ))
      )}
      <ActionButton
        label="Refresh authorizations"
        variant="secondary"
        disabled={busy || !isConnected}
        loading={apps.isFetching || bindings.isFetching}
        onPress={refresh}
      />
    </Section>
  );
}
