import { useState } from "react";
import { Alert, Linking, Switch } from "react-native";
import { useQuery } from "@tanstack/react-query";
import {
  SettingsGroup,
  SettingsRow,
} from "../../components/settings/SettingsList";
import { Notice } from "../../components/workspace/WorkspaceUI";
import { activeDomains } from "../../lib/domains";
import { orgGuestApi } from "../../lib/api/orgGuestApi";
import { useCollaborationSession } from "../groups/useCollaborationSession";
import type { AssistantAgent } from "./agentContracts";
import { GUEST_ACCESS_LABEL } from "./orgGuestContracts";

const EXPLANATION =
  "Guests are people who have not verified an organization member account on this bot. The organization pays for their messages and tool use. Verified members keep their own permissions and billing. Only organization admins can change guest access.";
function openGrants(id: string) {
  void Linking.openURL(
    `${activeDomains().frontendUrl}/assistant?agent=${encodeURIComponent(id)}&details=grants`,
  );
}

export function OrgGuestChannel({
  agent,
  channel,
}: {
  agent: AssistantAgent;
  channel: AssistantAgent["channels"][number];
}) {
  const [open, setOpen] = useState(false);
  const session = useCollaborationSession("nyxbot:org-specialist-channels");
  const query = useQuery({
    queryKey: [...session.key, "org-guest-settings", channel.id],
    queryFn: ({ signal }) => orgGuestApi.settings(channel.id, signal),
    enabled: open && session.active,
    retry: false,
  });
  const settings =
    session.readable &&
    query.dataUpdatedAt >= session.activationAt &&
    !query.isError
      ? query.data
      : undefined;
  return (
    <>
      <SettingsRow
        label={channel.bot_label}
        detail={channel.platform}
        value={channel.status}
        onPress={() => setOpen(!open)}
      />
      {open && (
        <SettingsGroup
          inset={false}
          title="Who can talk to this bot"
          footer={EXPLANATION}
        >
          <SettingsRow
            label="Allow guests"
            accessory={
              <Switch
                accessibilityLabel="Allow guests"
                value={settings?.guest_enabled ?? false}
                disabled={
                  !settings ||
                  !settings.can_manage ||
                  session.busy ||
                  channel.status !== "active" ||
                  (!settings.guest_enabled &&
                    (!settings.grants_ready ||
                      Boolean(settings.readiness_reason)))
                }
                onValueChange={(enabled) => {
                  if (settings)
                    void session.run(() =>
                      orgGuestApi.toggle(
                        channel.id,
                        enabled,
                        settings.guest_revision,
                        session.guard,
                      ),
                    );
                }}
              />
            }
          />
          {settings ? (
            <>
              {settings.guest_policy_mode === "legacy" ? (
                <SettingsRow
                  label="Existing guest limits are kept. Review them in Grants."
                  onPress={() => openGrants(agent.id)}
                />
              ) : (
                <>
                  <SettingsRow
                    label="Guests can use"
                    detail={
                      settings.effective_guest_services
                        .map((s) => s.label)
                        .join(", ") || "Model-only chat"
                    }
                  />
                  <SettingsRow
                    label="Enabling guests applies this specialist's Grants, including the default Use access."
                    onPress={() => openGrants(agent.id)}
                  />
                </>
              )}
              <SettingsRow
                label="Guest service access · Grants"
                onPress={() => openGrants(agent.id)}
              />
              <SettingsRow
                label="Organization spend today"
                value={`${settings.spent_today_credits} credits (${settings.reserved_today_credits} reserved)`}
                detail="This bot's guest spend today, including earlier connections. The organization pays."
              />
              {!settings.grants_ready && (
                <Notice message="Guest settings are upgrading. Existing service limits and the legacy daily cap still apply until every server is ready." />
              )}
              {!!settings.readiness_reason && (
                <Notice
                  message={
                    settings.readiness_reason === "org_wallet_suspended"
                      ? "The organization wallet is suspended."
                      : "Provision the organization billing wallet and subscription before enabling guests."
                  }
                />
              )}
            </>
          ) : (
            <Notice
              message={
                query.isError
                  ? "Could not load guest settings. Reopen to retry."
                  : "Loading guest settings…"
              }
            />
          )}
          {!!session.error && <Notice tone="error" message={session.error} />}
        </SettingsGroup>
      )}
    </>
  );
}

export function OrgGuestGrantsAdoption({
  agent,
  disabled,
}: {
  agent: AssistantAgent;
  disabled: boolean;
}) {
  const session = useCollaborationSession("nyxbot:org-specialist-channels");
  const query = useQuery({
    queryKey: [...session.key, "org-guest-adoption", agent.id],
    queryFn: ({ signal }) => orgGuestApi.preview(agent.id, signal),
    enabled: session.active,
    retry: false,
  });
  const preview =
    session.readable &&
    !query.isError &&
    query.dataUpdatedAt >= session.activationAt
      ? query.data
      : undefined;
  if (!preview?.pending && !query.isError) return null;
  return (
    <SettingsGroup
      inset={false}
      footer="Existing guest limits are kept. Save any Grants edits before reviewing the handoff."
    >
      <SettingsRow
        label="Use these guest settings for all linked organization bots"
        disabled={disabled || session.busy || !preview?.ready}
        onPress={() =>
          void session.run(async () => {
            const review = await orgGuestApi.preview(agent.id);
            session.guard();
            Alert.alert(
              "Use these guest settings for all linked organization bots",
              `This replaces each bot's existing service limits with saved Grants. Disabled bots use these settings if enabled later. The organization pays.\n\nAffected bots: ${review.bots.map((b) => b.name).join(", ")}\n\n${review.services.map((s) => `${s.label}: ${GUEST_ACCESS_LABEL[s.access]}${s.limited_operations ? " · Selected operations only" : ""}`).join("\n") || "Model-only chat"}`,
              [
                { text: "Cancel", style: "cancel" },
                {
                  text: "Apply reviewed guest settings",
                  onPress: () => {
                    if (session.valid())
                      void session.run(() =>
                        orgGuestApi.adopt(
                          agent.id,
                          review.review,
                          session.guard,
                        ),
                      );
                  },
                },
              ],
            );
          })
        }
      />
      {query.isError && (
        <Notice
          message="Could not load guest settings. Reload to review."
          tone="error"
        />
      )}
      {!!session.error && <Notice message={session.error} tone="error" />}
    </SettingsGroup>
  );
}
