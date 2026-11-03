import { useRef, useState } from "react";
import { Alert } from "react-native";
import * as WebBrowser from "expo-web-browser";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import {
  ActionButton,
  Card,
  Field,
  Label,
  Notice,
} from "../../components/workspace/WorkspaceUI";
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
  return (
    <Card>
      <Label accessibilityRole="header">Organizations</Label>
      <Label muted>
        Signed in as {email}. Your personal account remains the acting identity.
        Select an organization below to manage its own resources.
      </Label>
      {message ? <Notice {...message} /> : null}
      {orgs.isPending ? (
        <Label muted>Loading memberships…</Label>
      ) : orgs.isError ? (
        <Notice
          tone="error"
          message="Could not verify memberships. Organization actions are unavailable until refresh succeeds."
        />
      ) : orgs.data?.length === 0 ? (
        <Label muted>You are not a member of any organizations.</Label>
      ) : (
        orgs.data?.map((org) => (
          <ActionButton
            key={org.id}
            label={`${selectedId === org.id ? "Selected: " : ""}${org.display_name ?? org.slug} · ${org.your_role}`}
            variant="secondary"
            disabled={disabled}
            onPress={() => setSelectedId(org.id)}
          />
        ))
      )}
      {selected ? (
        <>
          <Label>
            Organization: {selected.display_name ?? selected.slug} · Role:{" "}
            {selected.your_role}
          </Label>
          <Label muted>
            Invitations, members/roles, role permissions, approval policy,
            profile changes and deletion continue on the website. Sign in there
            as {email}; organization access is verified by the server. Close the
            browser to return here and refresh.
          </Label>
          {ORGANIZATION_WEB_ACTIONS.filter(
            (action) => !action.adminOnly || admin,
          ).map((action) => (
            <ActionButton
              key={action.tab}
              label={`${action.label} on website`}
              variant="secondary"
              disabled={disabled}
              onPress={() => void openOrganization(action.tab)}
            />
          ))}
          {!admin ? (
            <Label muted>
              Only organization owners/admins can manage invitations, roles,
              approvals and settings.
            </Label>
          ) : null}
        </>
      ) : null}
      <Field
        label="New organization name"
        value={name}
        onChangeText={setName}
        editable={!disabled}
        autoCapitalize="words"
        maxLength={128}
      />
      <ActionButton
        label="Create organization"
        disabled={disabled || !name.trim()}
        onPress={() => void mutate("create")}
      />
      <Field
        label="Organization invitation code"
        value={nonce}
        onChangeText={setNonce}
        editable={!disabled}
        autoCorrect={false}
        secureTextEntry
      />
      <ActionButton
        label="Accept organization invitation"
        disabled={disabled || !nonce.trim()}
        onPress={() => {
          const ticket = current.capture();
          Alert.alert(
            "Join organization?",
            `Accept this invitation as ${email}? The server will verify its validity and assigned role.`,
            [
              { text: "Cancel", style: "cancel" },
              { text: "Join", onPress: () => void mutate("join", ticket) },
            ],
          );
        }}
      />
      <ActionButton
        label="Refresh memberships & permissions"
        variant="secondary"
        loading={orgs.isFetching}
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
    </Card>
  );
}
