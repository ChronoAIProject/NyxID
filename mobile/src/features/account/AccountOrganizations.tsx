import { useLocalizedState } from "../../i18n/useLocalizedState";
import { useAppLanguage } from "../../i18n/LanguageProvider";
import { translateApp } from "../../i18n/appTranslation";
import { useRef, useState } from "react";
import { Alert } from "react-native";
import * as WebBrowser from "expo-web-browser";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { SkeletonRows } from "../../components/Skeleton";
import { Notice } from "../../components/workspace/WorkspaceUI";
import {
  SettingsCheckRow,
  SettingsGroup,
  SettingsRow,
  SettingsTextField,
} from "../../components/settings/SettingsList";
import { accountApi } from "../../lib/api/accountApi";
import { resolveErrorMessage } from "../../lib/api/errorMessages";
import { getFrontendUrl } from "../../lib/env";
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
  useAppLanguage();
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
  const [message, setMessage] = useLocalizedState<{
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
        setMessage(() => ({
          message: (kind === "create" ? (listed
              ? translateApp("Organization created. Membership reloaded from NyxID.") : translateApp("Organization created. Membership could not reload yet; pull to refresh.")) : (listed ? translateApp("Invitation accepted. Membership reloaded from NyxID.") : translateApp("Invitation accepted. Membership could not reload yet; pull to refresh."))),
          tone: "success",
        }));
    } catch (error) {
      if (ticket.isCurrent())
        setMessage(() => ({ message: resolveErrorMessage(error), tone: "error" }));
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
          getFrontendUrl(),
          `/orgs/${encodeURIComponent(selected.id)}?tab=${tab}`,
        ),
      );
      // Closing the browser is only a return signal. Read live memberships again.
      if (current()) {
        const listed = await refresh();
        if (current())
          setMessage(() => ({
            message: listed
              ? translateApp(
                  "Returned from the website. Membership refreshed; no browser action is assumed completed.",
                )
              : translateApp(
                  "Returned from the website. Membership could not reload; pull to refresh.",
                ),
            tone: listed ? "info" : "error",
          }));
      }
    } catch (error) {
      if (current())
        setMessage(() => ({ message: resolveErrorMessage(error), tone: "error" }));
    } finally {
      locked.current = false;
      setBusy(false);
    }
  }
  const disabled = !isConnected || busy || orgs.isFetching;
  const joinWithCode = () => {
    const ticket = current.capture();
    Alert.alert(
      translateApp("Join organization?"),
      translateApp(
        "Accept this invitation as {{email}}? The server will verify its validity and assigned role.",
        { email: email },
      ),
      [
        { text: translateApp("Cancel"), style: "cancel" },
        { text: translateApp("Join"), onPress: () => void mutate("join", ticket) },
      ],
    );
  };
  return (
    <>
      {message ? <Notice {...message} /> : null}
      {orgs.isPending ? (
        <SkeletonRows
          rows={2}
          icon={false}
          title
          accessibilityLabel={translateApp("Loading memberships")}
        />
      ) : (
        <SettingsGroup
          title={translateApp("Organizations")}
          inset={false}
          footer={translateApp(
            "Signed in as {{email}}. Your personal account stays the acting identity; tap an organization to manage its resources.",
            { email: email },
          )}
        >
          {orgs.isError ? (
            <SettingsRow
              label={translateApp("Couldn't verify memberships")}
              detail={translateApp(
                "Organization actions are unavailable until refresh succeeds.",
              )}
            />
          ) : orgs.data?.length === 0 ? (
            <SettingsRow label={translateApp("Not a member of any organization")}
            />
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
            label={orgs.isFetching ? translateApp("Refreshing…")
                : translateApp("Refresh Memberships")
            }
            disabled={busy || !isConnected}
            onPress={() => {
              const ticket = current.capture();
              void refresh().catch((error) => {
                if (ticket.isCurrent())
                  setMessage(() => ({
                    message: resolveErrorMessage(error),
                    tone: "error",
                  }));
              });
            }}
          />
        </SettingsGroup>
      )}
      {selected ? (
        <SettingsGroup
          title={translateApp("Manage {{slug}}", {
            slug: selected.display_name ?? selected.slug,
          })}
          inset={false}
          footer={(admin
              ? translateApp("These open on the website. Sign in there as {{email}}; access is verified by the server.", { email: email }) : translateApp("These open on the website. Sign in there as {{email}}; access is verified by the server. Only owners and admins can manage invitations, roles, approvals and settings.", { email: email }))}
        >
          <SettingsRow label={translateApp("Your role")}
            value={selected.your_role} />
          {ORGANIZATION_WEB_ACTIONS.filter(
            (action) => !action.adminOnly || admin,
          ).map((action) => (
            <SettingsRow
              key={action.tab}
              label={action.label}
              value={translateApp("Website")}
              disabled={disabled}
              accessibilityHint={translateApp(
                "Opens {{label}} on the website",
                { label: action.label },
              )}
              onPress={() => void openOrganization(action.tab)}
            />
          ))}
        </SettingsGroup>
      ) : null}
      <SettingsGroup title={translateApp("Create organization")} inset={false}>
        <SettingsTextField
          accessibilityLabel={translateApp("New organization name")}
          placeholder={translateApp("Organization name")}
          value={name}
          onChangeText={setName}
          editable={!disabled}
          autoCapitalize="words"
          maxLength={128}
        />
        <SettingsRow
          tone="action"
          label={translateApp("Create Organization")}
          disabled={disabled || !name.trim()}
          onPress={() => void mutate("create")}
        />
      </SettingsGroup>
      <SettingsGroup
        title={translateApp("Join an organization")}
        inset={false}
        footer={translateApp(
          "Paste the invitation code an organization admin sent you.",
        )}
      >
        <SettingsTextField
          accessibilityLabel={translateApp("Organization invitation code")}
          placeholder={translateApp("Invitation code")}
          value={nonce}
          onChangeText={setNonce}
          editable={!disabled}
          autoCorrect={false}
          autoCapitalize="none"
          secureTextEntry
        />
        <SettingsRow
          tone="action"
          label={translateApp("Accept Invitation")}
          disabled={disabled || !nonce.trim()}
          onPress={joinWithCode}
        />
      </SettingsGroup>
    </>
  );
}
