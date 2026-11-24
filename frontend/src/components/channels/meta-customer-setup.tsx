import { getFormattingLocale } from "@/lib/formatting-locale";
import { OrgSpecialistChannelSettings } from "@/components/assistant/org-specialist-channel-settings";
import { useState } from "react";
import { z } from "zod";
import { zodResolver } from "@hookform/resolvers/zod";
import { useAppForm } from "@/components/ui/form";
import { Label } from "@/components/ui/label";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { ErrorBanner } from "@/components/shared/error-banner";
import { useUiTranslation } from "@/i18n/use-ui-translation";
import { useNyxBotAgents } from "@/hooks/use-nyxbot-agents";
import { useAuthStore } from "@/stores/auth-store";
import {
  type MetaCustomerSetupResult,
  useMetaCustomerSetup,
} from "@/hooks/use-channel-meta";

const schema = z.object({
  agent_id: z.string().min(1),
  enable_guests: z.boolean(),
});

interface SetupProps {
  readonly botId: string;
  readonly orgId: string | null;
}

export function MetaCustomerSetup(props: SetupProps) {
  const actor = useAuthStore((s) => s.user?.id);
  return <CustomerSetupForm key={`${actor ?? ""}:${props.botId}:${props.orgId ?? ""}`} {...props} />;
}

function CustomerSetupForm({ botId, orgId }: SetupProps) {
  const { t } = useUiTranslation();
  const agents = useNyxBotAgents();
  const save = useMetaCustomerSetup(botId);
  const [committed, setCommitted] = useState<MetaCustomerSetupResult | null>(
    null,
  );
  async function apply(
    action:
      | "check"
      | "assign"
      | "prepare_model"
      | "enable_guests"
      | "owner_link",
  ) {
    const result = await save.mutateAsync({
      agent_id: form.getValues("agent_id"),
      action,
    });
    setCommitted(result);
    if (result.owner_link_code)
      setOwnerLink({
        code: result.owner_link_code,
        expiresAt: result.owner_link_expires_at,
      });
  }
  const form = useAppForm({
    resolver: zodResolver(schema),
    defaultValues: { agent_id: "", enable_guests: false },
  });
  const [ownerLink, setOwnerLink] = useState<{
    code: string;
    expiresAt?: string | null;
  } | null>(null);
  const [checkedAgent, setCheckedAgent] = useState<string | null>(null);
  const specialists =
    agents.data?.agents.filter(
      (agent) =>
        agent.kind === "specialist" &&
        agent.status !== "destroyed" &&
        (orgId
          ? agent.owner_kind === "org" &&
            agent.owner_id === orgId &&
            agent.org_role === "admin"
          : agent.owner_kind !== "org" && !agent.share_role),
    ) ?? [];
  const agentId = form.watch("agent_id");
  const readiness = committed?.readiness;
  const ready =
    checkedAgent === agentId &&
    (orgId
      ? readiness?.grants_ready === true && !readiness.readiness_reason
      : readiness?.guest_readiness?.status === "ready");
  async function check() {
    if (!agentId) return;
    try {
      await apply(checkedAgent === agentId ? "check" : "assign");
      setCheckedAgent(agentId);
    } catch {
      setCheckedAgent(null);
    }
  }
  return (
    <section className="space-y-4 rounded-lg border border-border p-4">
      <h3 className="type-card-title">{t("Set up customer conversations")}</h3>
      <p className="type-caption text-muted-foreground">
        {t(
          "Private chats start with owner-only access. Select a support specialist and review its guest grants before opening this inbox to customers.",
        )}
      </p>
      <form
        className="space-y-3"
        onSubmit={form.handleSubmit(async (values) => {
          if (ready && values.enable_guests)
            await apply("enable_guests").catch(() => undefined);
        })}
      >
        <Label htmlFor={`meta-specialist-${botId}`}>
          {t("Support specialist")}
        </Label>
        <select
          id={`meta-specialist-${botId}`}
          className="w-full rounded-md border border-input bg-background p-2 type-body"
          value={agentId}
          disabled={save.isPending}
          onChange={(event) => {
            form.setValue("agent_id", event.target.value);
            form.setValue("enable_guests", false);
            setCheckedAgent(null);
            setCommitted(null);
            setOwnerLink(null);
            save.reset();
          }}
        >
          <option value="">{t("Choose a specialist")}</option>
          {specialists.map((agent) => (
            <option key={agent.id} value={agent.id}>
              {agent.display_name || agent.name}
            </option>
          ))}
        </select>
        {agents.isError && <ErrorBanner message={agents.error.message} />}
        {!agents.isPending && specialists.length === 0 && (
          <p className="type-caption text-muted-foreground">
            {t(
              "Create a specialist in the same personal or organization scope, then return here.",
            )}
          </p>
        )}
        <Button
          type="button"
          disabled={!agentId || save.isPending}
          onClick={() => void check()}
        >
          {checkedAgent === agentId
            ? t("Check readiness")
            : t("Assign specialist and check readiness")}
        </Button>
        {!orgId && checkedAgent === agentId && !ready && (
          <Button
            type="button"
            disabled={save.isPending}
            onClick={() => void apply("prepare_model").catch(() => undefined)}
          >
            {t("Prepare guest model")}
          </Button>
        )}
        {!orgId && checkedAgent === agentId && (
          <Button
            type="button"
            disabled={save.isPending}
            onClick={() => void apply("owner_link").catch(() => undefined)}
          >
            {t("Create a fresh owner verification code")}
          </Button>
        )}
        {orgId && committed?.channel_id && (
          <OrgSpecialistChannelSettings id={committed.channel_id} />
        )}
        {ownerLink && (
          <div
            role="region"
            aria-label={t("Owner verification")}
            className="space-y-2 rounded-md border border-border p-3"
          >
            <p className="type-caption text-muted-foreground">
              {t(
                "From your own private account, send this single-use code to the bot. Other customers must not receive it. A first DM alone never verifies the owner.",
              )}
            </p>
            <code className="type-code select-all">{ownerLink.code}</code>
            {ownerLink.expiresAt && (
              <p className="type-caption text-text-tertiary">
                {t("Expires at")}:{" "}
                {new Date(ownerLink.expiresAt).toLocaleString(
                  getFormattingLocale(),
                )}
              </p>
            )}
          </div>
        )}
        {readiness?.guest_readiness && (
          <p role="status" className="type-caption text-muted-foreground">
            {readiness.guest_readiness.status === "ready"
              ? t(
                  "Guest model is ready. Review the specialist's guest tool grants before continuing.",
                )
              : readiness.guest_readiness.fix ||
                t(
                  "Guest model setup is pending. Check the specialist's Model and Grants settings, then retry.",
                )}
          </p>
        )}
        {orgId && readiness && (
          <p role="status" className="type-caption text-muted-foreground">
            {!readiness.grants_ready
              ? t(
                  "Organization guest rollout is not ready. Ask an organization administrator to review channel readiness.",
                )
              : readiness.readiness_reason
                ? t(
                    "Provision and fund the organization wallet in Billing before enabling guests.",
                  )
                : t(
                    "Guest grants are configured and the organization wallet has an active subscription. Each guest turn must have enough funding.",
                  )}
          </p>
        )}
        <Label className="flex items-start gap-2">
          <Checkbox
            checked={form.watch("enable_guests")}
            disabled={!ready}
            onCheckedChange={(value) =>
              form.setValue("enable_guests", value === true)
            }
          />
          {orgId
            ? t(
                "Enable guest exposure and Everyone for private DMs. The organization pays for guests using this specialist's guest grants.",
              )
            : t(
                "Enable Everyone for private DMs using this specialist's guest grants. My account pays for guest model and tool use.",
              )}
        </Label>
        <Button
          type="submit"
          variant="primary"
          disabled={!ready || !form.watch("enable_guests") || save.isPending}
        >
          {t("Enable customer conversations")}
        </Button>
        {save.isError && <ErrorBanner message={save.error.message} />}
        {committed?.guests_enabled && (
          <p role="status" className="type-caption text-success">
            {t(
              "Customer conversations are enabled. Send a private text message from an unrelated test account and confirm the matching reply in the native inbox.",
            )}
          </p>
        )}
      </form>
    </section>
  );
}
