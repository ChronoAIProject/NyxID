import { useAppLanguage } from "../../i18n/LanguageProvider";
import { translateApp } from "../../i18n/appTranslation";
import { channelDetail } from "./channelPresentation";
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
  const { t } = useAppLanguage();
  const detail = channelDetail(channel);
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
  const canManage = settings?.can_manage && agent.can_maintain !== false;
  return (
    <>
      <SettingsRow
        label={channel.bot_label}
        detail={detail.kind === "message" ? t(detail.value) : detail.value}
        value={channel.status}
        onPress={() => setOpen(!open)}
      />
      {open && (
        <SettingsGroup
          inset={false}
          title={translateApp("Who can talk to this bot")}
          footer={translateApp(EXPLANATION)}
        >
          <SettingsRow
            label={translateApp("Allow guests")}
            accessory={
              <Switch
                accessibilityLabel={translateApp("Allow guests")}
                value={settings?.guest_enabled ?? false}
                disabled={
                  !settings ||
                  !canManage ||
                  session.busy ||
                  channel.status !== "active" ||
                  (!settings.guest_enabled &&
                    (!settings.grants_ready ||
                      Boolean(settings.readiness_reason)))
                }
                onValueChange={(enabled) => {
                  if (settings && canManage && !session.busy)
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
              {!canManage && <Notice message={t("An organization admin can change these settings.")} />}
              {(settings.guest_policy_mode === "grants" || settings.binding_limits_guest_services) && (
                <SettingsRow
                  label={translateApp("Guests can use")}
                  detail={
                    settings.effective_guest_services.map((s) => s.label).join(", ") ||
                    translateApp("Model-only chat")
                  }
                />
              )}
              {settings.guest_policy_mode === "legacy" ? (
                <SettingsRow
                  label={settings.binding_limits_guest_services
                    ? translateApp("This bot's saved limits exclude services listed for guests in the specialist's Grants. An organization admin can review access for all linked bots in Grants.")
                    : translateApp("Existing guest limits are kept. Review them in Grants.")}
                  onPress={() => openGrants(agent.id)}
                />
              ) : (
                <SettingsRow
                  label={translateApp(
                    "Enabling guests applies this specialist's Grants, including the default Use access.",
                  )}
                  onPress={() => openGrants(agent.id)}
                />
              )}
              <SettingsRow
                label={translateApp("Guest service access · Grants")}
                onPress={() => openGrants(agent.id)}
              />
              {settings.can_manage && settings.spent_today_credits !== undefined && settings.reserved_today_credits !== undefined && <SettingsRow
                label={translateApp("Organization spend today")}
                value={translateApp(
                  "{{spent}} credits ({{reserved}} reserved)",
                  {
                    spent: settings.spent_today_credits,
                    reserved: settings.reserved_today_credits,
                  },
                )}
                detail={translateApp(
                  "This bot's guest spend today, including earlier connections. The organization pays.",
                )}
              />}
              {!settings.grants_ready && (
                <Notice message={translateApp(
                    "Guest settings are upgrading. Existing service limits and the legacy daily cap still apply until every server is ready.",
                  )}
                />
              )}
              {!!settings.readiness_reason && (
                <Notice
                  message={
                    settings.readiness_reason === "org_wallet_suspended"
                      ? translateApp("The organization wallet is suspended.")
                      : translateApp("Set up organization billing to serve guests.")
                  }
                />
              )}
            </>
          ) : (
            <Notice
              message={
                query.isError
                  ? translateApp(
                      "Could not load guest settings. Reopen to retry.",
                    )
                  : translateApp("Loading guest settings…")
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
  useAppLanguage();
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
      footer={translateApp(
        "Existing guest limits are kept. Save any Grants edits before reviewing the handoff.",
      )}
    >
      <SettingsRow
        label={translateApp(
          "Use these guest settings for all linked organization bots",
        )}
        disabled={disabled || session.busy || !preview?.ready}
        onPress={() =>
          void session.run(async () => {
            const review = await orgGuestApi.preview(agent.id);
            session.guard();
            Alert.alert(
              translateApp(
                "Use these guest settings for all linked organization bots",
              ),
              translateApp(
                "This replaces each bot's existing service limits with saved Grants. Disabled bots use these settings if enabled later. The organization pays.\n\nAffected bots: {{name}}\n\n{{label}}",
                {
                  name: review.bots.map((b) => b.name).join(", "),
                  label:
                    review.services.map((s) => `${s.label}: ${GUEST_ACCESS_LABEL[s.access]}${s.limited_operations ? " · Selected operations only" : ""}`).join("\n") || "Model-only chat",
                },
              ),
              [
                { text: translateApp("Cancel"), style: "cancel" },
                {
                  text: translateApp("Apply reviewed guest settings"),
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
          message={translateApp(
            "Could not load guest settings. Reload to review.",
          )}
          tone="error"
        />
      )}
      {!!session.error && <Notice message={session.error} tone="error" />}
    </SettingsGroup>
  );
}
