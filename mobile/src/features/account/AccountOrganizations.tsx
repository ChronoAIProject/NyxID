import { useRef, useState } from "react";
import { Alert } from "react-native";
import * as WebBrowser from "expo-web-browser";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Notice } from "../../components/workspace/WorkspaceUI";
import {
  SettingsCheckRow,
  SettingsGroup,
  SettingsRow,
  SettingsTextField,
} from "../../components/settings/SettingsList";
import { accountApi } from "../../lib/api/accountApi";
import { resolveErrorMessage } from "../../lib/api/errorMessages";
import { NYXID_FRONTEND_URL } from "../../lib/env";
import { useNetworkStatus } from "../../hooks/useNetworkStatus";
import {
  useAccountSecurityRefresh,
  type AccountSecurityTicket,
} from "./useAccountSecurityRefresh";
import { accountWebUrl, ORGANIZATION_WEB_ACTIONS } from "./accountWebHandoff";

export function AccountOrganizations({
  accountId,
  email,
}: {
  accountId: string;
  email: string;
}) {
  const client = useQueryClient();
  const { isConnected } = useNetworkStatus();
  const key = ["account", "organizations", accountId];
  const orgs = useQuery({
    queryKey: key,
    queryFn: ({ signal }) => accountApi.organizations(accountId, signal),
  });
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [name, setName] = useState("");
  const [nonce, setNonce] = useState("");
  const [busy, setBusy] = useState(false);
  const locked = useRef(false);
  const [message, setMessage] = useState<{
    message: string;
    tone: "info" | "success" | "error";
  } | null>(null);
  const reloadOrganizations = () =>
    client.resetQueries({ queryKey: key }, { throwOnError: true });
  /** Reload after a membership change; reports whether the list reloaded. */
  const refresh = async () => {
    // Membership changes invalidate inherited resource permissions, not personal ownership.
    const [listed] = await Promise.allSettled([
      reloadOrganizations(),
      client.resetQueries({ queryKey: ["workspace", "services"] }),
      client.resetQueries({ queryKey: ["workspace", "agents"] }),
      client.resetQueries({ queryKey: ["workspace", "groups"] }),
    ]);
    return listed.status === "fulfilled";
  };
  const current = useAccountSecurityRefresh(
    () => {
      // Viewing the screen changes no membership; keep workspace caches intact.
      void reloadOrganizations().catch(() => {});
    },
    () => {
      setNonce("");
      setMessage(null);
    },
  );
  const selected = !orgs.isError
    ? orgs.data?.find((org) => org.id === selectedId)
    : undefined;
  const admin =
    selected?.your_role === "owner" || selected?.your_role === "admin";
  async function mutate(
    kind: "create" | "join",
    ticket: AccountSecurityTicket = current.capture(),
  ) {
    if (locked.current || !ticket.isCurrent()) return;
    locked.current = true;
    setBusy(true);
    setMessage(null);
    try {
      const guard = { authorityGuard: ticket.assertCurrent };
      const id =
        kind === "create"
          ? (await accountApi.createOrganization(accountId, name, guard)).id
          : (await accountApi.joinOrganization(accountId, nonce.trim(), guard))
              .org_id;
      if (!ticket.isCurrent()) return;
      setName("");
      setNonce("");
      setSelectedId(id);
      const listed = await refresh();
      if (ticket.isCurrent())
        setMessage({
          message: `${
            kind === "create" ? "Organization created." : "Invitation accepted."
          } ${
            listed
              ? "Membership reloaded from NyxID."
              : "Membership could not reload yet; pull to refresh."
          }`,
          tone: "success",
        });
    } catch (error) {
      if (ticket.isCurrent())
        setMessage({ message: resolveErrorMessage(error), tone: "error" });
    } finally {
      locked.current = false;
      setBusy(false);
    }
  }
  async function openOrganization(tab: string) {
    if (!selected || locked.current || !current()) return;
    locked.current = true;
    setBusy(true);
    setMessage(null);
    try {
      await WebBrowser.openBrowserAsync(
        accountWebUrl(
          NYXID_FRONTEND_URL,
          `/orgs/${encodeURIComponent(selected.id)}?tab=${tab}`,
        ),
      );
      // Closing the browser is only a return signal. Read live memberships again.
      if (current()) {
        const listed = await refresh();
        if (current())
          setMessage({
            message: listed
              ? "Returned from the website. Membership refreshed; no browser action is assumed completed."
              : "Returned from the website. Membership could not reload; pull to refresh.",
            tone: listed ? "info" : "error",
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
  const disabled = !isConnected || busy || orgs.isFetching;
  const joinWithCode = () => {
    const ticket = current.capture();
    Alert.alert(
      "Join organization?",
      `Accept this invitation as ${email}? The server will verify its validity and assigned role.`,
      [
        { text: "Cancel", style: "cancel" },
        { text: "Join", onPress: () => void mutate("join", ticket) },
      ],
    );
  };
  return (
    <>
      {message ? <Notice {...message} /> : null}
      <SettingsGroup
        title="Organizations"
        inset={false}
        footer={`Signed in as ${email}. Your personal account stays the acting identity; tap an organization to manage its resources.`}
      >
        {orgs.isPending ? (
          <SettingsRow label="Loading memberships…" />
        ) : orgs.isError ? (
          <SettingsRow
            label="Couldn't verify memberships"
            detail="Organization actions are unavailable until refresh succeeds."
          />
        ) : orgs.data?.length === 0 ? (
          <SettingsRow label="Not a member of any organization" />
        ) : (
          orgs.data?.map((org) => (
            <SettingsCheckRow
              key={org.id}
              label={org.display_name ?? org.slug}
              detail={org.your_role}
              selected={selectedId === org.id}
              disabled={disabled}
              onPress={() => setSelectedId(org.id)}
            />
          ))
        )}
        <SettingsRow
          tone="action"
          label={orgs.isFetching ? "Refreshing…" : "Refresh Memberships"}
          disabled={busy || !isConnected}
          onPress={() => {
            const ticket = current.capture();
            void refresh().catch((error) => {
              if (ticket.isCurrent())
                setMessage({
                  message: resolveErrorMessage(error),
                  tone: "error",
                });
            });
          }}
        />
      </SettingsGroup>
      {selected ? (
        <SettingsGroup
          title={`Manage ${selected.display_name ?? selected.slug}`}
          inset={false}
          footer={`These open on the website. Sign in there as ${email}; access is verified by the server.${
            admin
              ? ""
              : " Only owners and admins can manage invitations, roles, approvals and settings."
          }`}
        >
          <SettingsRow label="Your role" value={selected.your_role} />
          {ORGANIZATION_WEB_ACTIONS.filter(
            (action) => !action.adminOnly || admin,
          ).map((action) => (
            <SettingsRow
              key={action.tab}
              label={action.label}
              value="Website"
              disabled={disabled}
              accessibilityHint={`Opens ${action.label} on the website`}
              onPress={() => void openOrganization(action.tab)}
            />
          ))}
        </SettingsGroup>
      ) : null}
      <SettingsGroup title="Create organization" inset={false}>
        <SettingsTextField
          accessibilityLabel="New organization name"
          placeholder="Organization name"
          value={name}
          onChangeText={setName}
          editable={!disabled}
          autoCapitalize="words"
          maxLength={128}
        />
        <SettingsRow
          tone="action"
          label="Create Organization"
          disabled={disabled || !name.trim()}
          onPress={() => void mutate("create")}
        />
      </SettingsGroup>
      <SettingsGroup
        title="Join an organization"
        inset={false}
        footer="Paste the invitation code an organization admin sent you."
      >
        <SettingsTextField
          accessibilityLabel="Organization invitation code"
          placeholder="Invitation code"
          value={nonce}
          onChangeText={setNonce}
          editable={!disabled}
          autoCorrect={false}
          autoCapitalize="none"
          secureTextEntry
        />
        <SettingsRow
          tone="action"
          label="Accept Invitation"
          disabled={disabled || !nonce.trim()}
          onPress={joinWithCode}
        />
      </SettingsGroup>
    </>
  );
}
