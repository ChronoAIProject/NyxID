import { useLocalizedState } from "../../i18n/useLocalizedState";
import { useAppLanguage } from "../../i18n/LanguageProvider";
import { translateApp } from "../../i18n/appTranslation";
import { useRef, useState } from "react";
import { Alert } from "react-native";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { SkeletonRows } from "../../components/Skeleton";
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
  useAppLanguage();
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
  const [feedback, setFeedback] = useLocalizedState<{
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
        setFeedback(() => ({
          message: listed
            ? translateApp("Access revoked. Lists refreshed from NyxID.")
            : translateApp(
                "Access revoked. Lists could not reload yet; retry to confirm.",
              ),
          tone: "success",
        }));
    } catch (error) {
      if (ticket.isCurrent())
        setFeedback(() => ({ message: resolveErrorMessage(error), tone: "error" }));
    } finally {
      locked.current = false;
      setBusy(false);
    }
  }
  function confirm(kind: "app" | "binding", id: string, label: string) {
    const ticket = current.capture();
    Alert.alert(
      translateApp("Revoke {{label}}?", { label: label}),
      kind === "app"
        ? translateApp(
            "This application will need consent again. Related Machine permissions are revoked with this consent.",
          )
        : translateApp(
            "This app will need authorization again before it can act on your behalf.",
          ),
      [
        { text: translateApp("Cancel"), style: "cancel" },
        {
          text: translateApp("Revoke"),
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
      ? translateApp("Legacy unrestricted service access")
      : app.allow_all_services
        ? translateApp("All services")
        : app.allowed_services
            .map(
              (service) =>
                (service.deleted ? translateApp("{{service}} (deleted)", { service: service.label}) : service.label),
            )
            .join(", ") || translateApp("No services");
  return (
    <>
      {feedback ? <Notice {...feedback} /> : null}
      {apps.isPending ? (
        <SkeletonRows
          rows={2}
          icon={false}
          title
          accessibilityLabel={translateApp("Loading authorized apps")}
        />
      ) : (
        <SettingsGroup
          title={translateApp("Authorized apps")}
          inset={false}
          footer={translateApp(
            "Apps you signed in to with NyxID. Tap one to revoke it. These are separate from approval grants in Activity.",
          )}
        >
          {apps.isError ? (
            <SettingsRow
              label={translateApp("Couldn't load authorized apps")}
              detail={translateApp("Tap to retry.")}
              tone="action"
              onPress={refresh}
            />
          ) : apps.data?.length === 0 ? (
            <SettingsRow label={translateApp("No authorized apps")} />
          ) : (
            apps.data?.map((app) => (
              <SettingsRow
                key={app.client_id}
                label={app.client_name}
                detail={translateApp(
                  "{{services}} · {{details}}\nGranted {{grantedAt}} · Expires {{date}}",
                  {
                    services: services(app),
                    details: app.scopes || "No scopes",
                    grantedAt: app.granted_at,
                    date: app.expires_at ?? "never",
                  },
                )}
                value={translateApp("Revoke")}
                disabled={disabled}
                accessibilityHint={translateApp("Revoke {{service}}", {
                  service: app.client_name,
                })}
                onPress={() => confirm("app", app.client_id, app.client_name)}
              />
            ))
          )}
        </SettingsGroup>
      )}
      {bindings.isPending ? (
        <SkeletonRows
          rows={2}
          icon={false}
          title
          accessibilityLabel={translateApp("Loading broker authorizations")}
        />
      ) : (
        <SettingsGroup
          title={translateApp("Broker authorizations")}
          inset={false}
          footer={translateApp("Apps acting for you through NyxID's broker.")}
        >
          {bindings.isError ? (
            <SettingsRow
              label={translateApp("Couldn't load broker authorizations")}
              detail={translateApp("Tap to retry.")}
              tone="action"
              onPress={refresh}
            />
          ) : bindings.data?.length === 0 ? (
            <SettingsRow label={translateApp("No broker authorizations")} />
          ) : (
            bindings.data?.map((binding) => {
              const name = binding.client_name ?? binding.client_id;
              return (
                <SettingsRow
                  key={binding.binding_hash}
                  label={name}
                  detail={(binding.external_subject
                      ? translateApp("{{platform}} · {{externalUserId}}\n{{scopes}}\nCreated {{createdAt}} · Last used {{lastUsedAt}}", { platform: binding.external_subject.platform, externalUserId: binding.external_subject.external_user_id, scopes: binding.scopes.join(" "), createdAt: binding.created_at, lastUsedAt: binding.last_used_at ?? "never"}) : translateApp("{{scopes}}\nCreated {{createdAt}} · Last used {{lastUsedAt}}", { scopes: binding.scopes.join(" "), createdAt: binding.created_at, lastUsedAt: binding.last_used_at ?? "never" }))}
                  value={translateApp("Revoke")}
                  disabled={disabled}
                  accessibilityHint={translateApp("Revoke {{name}}", { name: binding.client_name ?? translateApp("authorization") })}
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
                ? translateApp("Refreshing…")
                : translateApp("Refresh Authorizations")
            }
            disabled={busy || !isConnected}
            onPress={refresh}
          />
        </SettingsGroup>
      )}
    </>
  );
}
