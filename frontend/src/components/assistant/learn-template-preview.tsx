import { useEffect, useRef, useState } from "react";
import { Link, useNavigate } from "@tanstack/react-router";
import { ArrowRight } from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogClose,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  DialogTrigger,
} from "@/components/ui/dialog";
import { useUiTranslation } from "@/i18n/use-ui-translation";
import { readSkillPreview, useAttachTemplateSkill } from "@/hooks/use-agent-skills";
import { ApiError } from "@/lib/api-client";
import { getAssistantIdentityRevision } from "@/lib/assistant/identity";
import { nyxBotApi } from "@/lib/assistant/nyxbot-api";
import { useAssistantDraftStore } from "@/stores/assistant-draft-store";
import { type SkillTemplate } from "@/lib/assistant/skill-templates";
import { useAuthStore } from "@/stores/auth-store";

const focusRing = "focus-visible:ring-2 focus-visible:ring-input-focus focus-visible:ring-offset-2 focus-visible:ring-offset-background";

type ConfirmationStep = "preview" | "agent" | "attach";
type TemplateFailure = "access" | "mismatch" | "full" | "disabled" | "different" | "conflict" | "agent" | "retry";

function confirmationFailure(error: unknown, step: ConfirmationStep): TemplateFailure {
  if (!(error instanceof ApiError)) return "retry";
  const details = error.errorResponse.details;
  if (details && typeof details === "object" && "reason" in details) {
    if (details.reason === "pinned_version_mismatch") return "mismatch";
    if (details.reason === "ornn_principal_denied" || details.reason === "owner_connection_unavailable") return "access";
  }
  if (error.status === 409) return "conflict";
  if (step === "agent") return "agent";
  // Catalog preview and skill attachment both enforce the owner's Ornn access.
  if (error.status === 403 || (step === "preview" && [401, 404].includes(error.status))) return "access";
  return "retry";
}

export function LearnTemplatePreview({ template, featured = false, available, owner }: {
  readonly template: SkillTemplate;
  readonly owner: string | undefined;
  readonly featured?: boolean;
  readonly available: boolean;
}) {
  const { t } = useUiTranslation();
  const navigate = useNavigate();
  const title = template.title();

  const [open, setOpen] = useState(false);
  const [busy, setBusy] = useState(false);
  const [failure, setFailure] = useState<TemplateFailure>();
  const [agentId, setAgentId] = useState<string>();
  const attempt = useRef<symbol | null>(null);
  const attach = useAttachTemplateSkill();
  const starter = t("Use the Ornn skill {{skill}} (version {{version}}) for this task. {{starter}} Ask me for any missing inputs ({{inputs}}).", {
    skill: template.slug,
    version: template.version,
    starter: template.description(),
    inputs: template.inputs().map(([label]) => label).join(", "),
  });

  useEffect(() => () => { attempt.current = null; }, [owner, template.slug]);
  function changeOpen(next: boolean) {
    if (next && attempt.current) return;
    if (!next) attempt.current = null;
    setBusy(false);
    setFailure(undefined);
    setOpen(next);
  }

  async function confirm() {
    if (!owner || !available || attempt.current || owner !== useAuthStore.getState().user?.id) return;
    const active = Symbol();
    const revision = getAssistantIdentityRevision();
    attempt.current = active;
    const guard = () => {
      if (attempt.current !== active || owner !== useAuthStore.getState().user?.id ||
        revision !== getAssistantIdentityRevision()) throw new DOMException("Template selection changed", "AbortError");
    };
    setBusy(true);
    setFailure(undefined);
    let step: ConfirmationStep = "preview";
    try {
      const { reference: pin } = await readSkillPreview(template.skillId, template.version, guard);
      guard();
      if (pin.disabled || pin.skill_id !== template.skillId || pin.version !== template.version ||
          pin.sha256 !== template.sha256) {
        setFailure("mismatch");
        return;
      }
      step = "agent";
      const { agents } = await nyxBotApi.agents(false, guard);
      guard();
      const personal = agents.find((agent) => agent.kind === "nyxbot" && !agent.share_role &&
        agent.owner_kind !== "org" && (!agent.owner_id || agent.owner_id === owner) &&
        agent.status !== "destroyed" && agent.can_use !== false);
      if (!personal) { setFailure("agent"); return; }
      setAgentId(personal.id);
      step = "attach";
      const result = await attach(personal.id, pin, guard);
      guard();
      if (result !== "ready") {
        setFailure(result);
        return;
      }
      const id = crypto.randomUUID();
      useAssistantDraftStore.getState().saveDraft(owner, `screen:nyxagent:learn:${id}`, starter);
      await navigate({ to: "/assistant", search: { agent: personal.id, draft: true, learnDraft: id } });
    } catch (error) {
      if (attempt.current !== active || owner !== useAuthStore.getState().user?.id ||
          revision !== getAssistantIdentityRevision()) return;
      setFailure(confirmationFailure(error, step));
    } finally {
      if (attempt.current === active) { attempt.current = null; setBusy(false); }
    }
  }
  const messages = {
    access: t("Your account can’t open this Ornn skill. Check your Ornn access or choose another template."),
    mismatch: t("NyxBot could not verify the published package against this template’s pinned version. Choose another template."),
    disabled: t("This skill is disabled on your NyxBot. Enable the pinned version in Skills to use this template."),
    different: t("Your NyxBot has a different pin for this skill. Review it in Skills to use this template’s version."),
    conflict: t("Your NyxBot skills changed while starting this chat. Try again to use the latest list."),
    full: t("Your NyxBot has 16 skills. Remove one in Skills to make room for this template."),
    agent: t("Could not load your personal NyxBot. Try again."),
    retry: t("Could not add the skill to NyxBot. Try again."),
  };
  return (
    <Dialog open={open} onOpenChange={changeOpen}>
      <DialogTrigger asChild>
        <Button type="button" variant={featured ? "primary" : "outline"} className={focusRing} aria-label={t("Try this · {{title}}", { title })}>
          {t("Try this")} <ArrowRight aria-hidden="true" />
        </Button>
      </DialogTrigger>
      <DialogContent className="md:max-w-xl [&>[data-icon-button]:focus-visible]:ring-2 [&>[data-icon-button]:focus-visible]:ring-input-focus">
        <DialogHeader className="pr-8">
          <DialogTitle>{title}</DialogTitle>
          <DialogDescription>{template.description()}</DialogDescription>
        </DialogHeader>
        <div className="rounded-lg border border-hairline bg-card p-4">
          <h3 className="type-row-title">{t("Starter message")}</h3>
          <p className="mt-2 break-words text-15 text-muted-foreground">{starter}</p>
        </div>
        {available ? (
          <p className="text-14 text-muted-foreground">
            {t("Continuing adds this skill to your personal NyxBot and opens a private chat with a starter message you can edit before sending.")}{" "}
            {t("Once added, the skill stays in your NyxBot’s Skills list, even if you leave the chat.")}
          </p>
        ) : <p role="status" className="text-14 text-muted-foreground">{t("This template requires NyxBot, which is unavailable for this account.")}</p>}
        {failure && <p role="alert" className="text-14 text-destructive">{messages[failure]}</p>}
        <DialogFooter>
          <DialogClose asChild><Button type="button" variant="outline" className={focusRing}>{t("Cancel")}</Button></DialogClose>
          {(failure === "full" || failure === "disabled" || failure === "different") && agentId ? (
            <Button asChild><Link to="/assistant" search={{ agent: agentId, details: "skills" }}>{t("Manage NyxBot skills")}</Link></Button>
          ) : failure === "mismatch" || failure === "access" ? (
            <DialogClose asChild><Button type="button">{t("Choose another template")}</Button></DialogClose>
          ) : available && (
            <Button type="button" variant="primary" className={focusRing} onClick={() => void confirm()} isLoading={busy} disabled={!owner || busy}>
              {t("Open NyxBot")} <ArrowRight aria-hidden="true" />
            </Button>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
