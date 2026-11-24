import { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";
import { Link, useRouter } from "@tanstack/react-router";
import { ArrowLeft, ArrowRight } from "lucide-react";
import { NyxidIcon } from "@/components/brand/nyxid-icon";
import { Button } from "@/components/ui/button";
import { useAttachTemplateSkill, useSkillPreview } from "@/hooks/use-agent-skills";
import { useApplyTheme } from "@/hooks/use-theme";
import { useUiTranslation } from "@/i18n/use-ui-translation";
import { ApiError } from "@/lib/api-client";
import { getAssistantIdentityRevision } from "@/lib/assistant/identity";
import { nyxBotApi } from "@/lib/assistant/nyxbot-api";
import { parseAssistantSearch } from "@/lib/assistant/search";
import { claimSkillSetup, readOpenTemplateDraft, saveOpenTemplateDraft } from "@/lib/assistant/skill-template-handoff";
import type { SkillTemplate } from "@/lib/assistant/skill-templates";
import type { AgentSkill } from "@/schemas/agent-skills";
import { useAuthStore } from "@/stores/auth-store";
import { useAssistantDraftStore } from "@/stores/assistant-draft-store";
import { NyxAgentAssistantChatPage, type TemplateDraft } from "./assistant-chat-page";

type PreviewFailure = "preview_access" | "preview_temporary" | "preview_mismatch";
type StartFailure = PreviewFailure | "agent" | "attach" | "conflict" | "disabled" | "different" | "full" | "missing";

function matchesTemplate(reference: AgentSkill | undefined, template: SkillTemplate) {
  return reference && !reference.disabled && reference.skill_id === template.skillId &&
    reference.version === template.version && reference.sha256 === template.sha256;
}

function previewFailure(error: unknown): PreviewFailure {
  if (error instanceof ApiError) {
    const details = error.errorResponse.details as { reason?: unknown } | undefined;
    if (details?.reason === "pinned_version_mismatch" ||
      (error.status === 400 && error.errorResponse.error === "validation_error")) {
      return "preview_mismatch";
    }
    if ([401, 403, 404].includes(error.status) || details?.reason === "ornn_principal_denied" ||
      details?.reason === "owner_connection_unavailable") {
      return "preview_access";
    }
  }
  return "preview_temporary";
}

export function SkillTemplateChat({ template, available, setupId }: {
  readonly template: SkillTemplate;
  readonly available: boolean;
  readonly setupId?: string;
}) {
  const [draft, setDraft] = useState<TemplateDraft>();
  const showDraft = useCallback((opened: TemplateDraft) => setDraft(opened), []);

  return draft ? <NyxAgentAssistantChatPage templateDraft={draft} /> :
    <SkillTemplatePreview template={template} available={available} setupId={setupId} onOpen={showDraft} />;
}

function SkillTemplatePreview({ template, available, setupId, onOpen }: {
  readonly template: SkillTemplate;
  readonly available: boolean;
  readonly setupId?: string;
  readonly onOpen: (draft: TemplateDraft) => void;
}) {
  const { t } = useUiTranslation();
  useApplyTheme("assistant");
  const router = useRouter();
  const owner = useAuthStore((state) => state.user?.id);
  const preview = useSkillPreview(available ? template.skillId : undefined, template.version);
  const attach = useAttachTemplateSkill();
  const [starting, setStarting] = useState(false);
  const [failure, setFailure] = useState<StartFailure>();
  const [skillsAgentId, setSkillsAgentId] = useState<string>();
  const automaticChecked = useRef(false);
  const mounted = useRef(false);
  const attempt = useRef<{ revision: number } | null>(null);
  useLayoutEffect(() => {
    mounted.current = true;
    return () => { mounted.current = false; };
  }, []);

  function attemptIsCurrent(active: { revision: number }) {
    const location = router.state.location;
    return mounted.current && attempt.current === active &&
      owner === useAuthStore.getState().user?.id && active.revision === getAssistantIdentityRevision() &&
      location.pathname.replace(/\/+$/, "") === "/assistant" &&
      parseAssistantSearch(location.search).skill === template.slug;
  }

  const starter = t("Use the Ornn skill {{skill}} (version {{version}}) for this task. {{starter}}", {
    skill: template.slug,
    version: template.version,
    starter: template.starter(),
  });
  const verified = !preview.isError && matchesTemplate(preview.data?.reference, template);

  async function start(restoringAgentId?: string) {
    if (!available || !owner || !verified || attempt.current) return;
    const active = { revision: getAssistantIdentityRevision() };
    attempt.current = active;
    const guard = () => {
      if (!attemptIsCurrent(active)) throw new DOMException("Template selection changed", "AbortError");
    };
    setStarting(true);
    setFailure(undefined);
    let stage: "preview" | "agent" | "attach" = "preview";
    try {
      const latest = await preview.refetch({ throwOnError: true, cancelRefetch: false });
      guard();
      const reference = latest.data?.reference;
      if (!reference || !matchesTemplate(reference, template)) {
        setFailure("preview_mismatch");
        return;
      }

      stage = "agent";
      const { agents } = await nyxBotApi.agents(false, guard);
      const agent = agents.find((row) => (!restoringAgentId || row.id === restoringAgentId) &&
        row.kind === "nyxbot" && !row.share_role && row.status !== "destroyed" &&
        row.can_use !== false && row.owner_kind !== "org" &&
        (row.owner_id === undefined || row.owner_id === owner));
      if (!agent) {
        setFailure("agent");
        return;
      }

      stage = "attach";
      const status = await attach(agent.id, reference, guard, !restoringAgentId);
      guard();
      if (status !== "ready") {
        setSkillsAgentId(agent.id);
        setFailure(status);
        return;
      }

      const key = `screen:nyxagent:template:${template.slug}:${setupId ?? crypto.randomUUID()}`;
      if (!restoringAgentId) {
        const store = useAssistantDraftStore.getState();
        if (store.ownerUserId !== owner || !Object.hasOwn(store.drafts, key)) store.saveDraft(owner, key, starter);
        if (setupId) saveOpenTemplateDraft({ skill: template.slug, skillSetup: setupId }, owner, agent.id);
      }
      onOpen({ key, setupId, title: template.title(), agent: { id: agent.id, name: agent.name, kind: "nyxbot", destroyed: false } });
    } catch (error) {
      if (!attemptIsCurrent(active)) return;
      if (stage === "preview") {
        setFailure(previewFailure(error));
      } else {
        setFailure(stage === "attach" && error instanceof ApiError && error.status === 409 ? "conflict" : stage);
      }
    } finally {
      if (attemptIsCurrent(active)) setStarting(false);
      if (attempt.current === active) attempt.current = null;
    }
  }

  useEffect(() => {
    if (!setupId || !owner || automaticChecked.current || (available && preview.isPending)) return;
    automaticChecked.current = true;
    const setup = { skill: template.slug, skillSetup: setupId };
    const claimed = claimSkillSetup(setup, owner);
    const restoringAgentId = readOpenTemplateDraft(setup, owner);
    if (available && verified && (claimed || restoringAgentId)) void start(restoringAgentId);
    // `start` closes over the verified preview for this exact render. The
    // synchronous claim and per-mount guard keep rerenders from retrying it.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [available, owner, preview.isPending, setupId, template.slug, verified]);

  const messages: Record<StartFailure, string> = {
    preview_access: t("Your account can’t open this Ornn skill. Check your Ornn access or choose another template."),
    preview_temporary: t("The Ornn catalog is temporarily unavailable. Try verification again."),
    preview_mismatch: t("NyxBot could not verify the published package against this template’s pinned version. Choose another template."),
    agent: t("Could not load your personal NyxBot. Try again."),
    attach: t("Could not add the skill to NyxBot. Try again."),
    conflict: t("Your NyxBot skills changed while starting this chat. Try again to use the latest list."),
    disabled: t("This skill is disabled on your NyxBot. Enable the pinned version in Skills to use this template."),
    different: t("Your NyxBot has a different pin for this skill. Review it in Skills to use this template’s version."),
    full: t("Your NyxBot has 16 skills. Remove one in Skills to make room for this template."),
    missing: t("This skill was removed from NyxBot. Add it again to continue with this template."),
  };
  const verificationIssue = preview.isError ? previewFailure(preview.error) :
    preview.data && !verified ? "preview_mismatch" : undefined;
  const issue = failure ?? verificationIssue;
  const manageSkills = failure === "disabled" || failure === "different" || failure === "full";

  return (
    <div className="min-h-dvh bg-background text-foreground">
      <header className="border-b border-hairline">
        <div className="mx-auto flex max-w-3xl items-center justify-between gap-4 px-5 py-4 sm:px-8">
          <Button asChild variant="ghost"><Link to="/testing" search={{ skill: template.slug }}><ArrowLeft aria-hidden="true" />{t("Skill templates")}</Link></Button>
          <Link to="/assistant" search={{}} className="flex items-center gap-2 rounded type-button focus-visible:outline-2 focus-visible:outline-offset-4 focus-visible:outline-primary">
            <NyxidIcon className="size-5" alt="" /> {t("NyxBot")}
          </Link>
        </div>
      </header>
      <main className="mx-auto max-w-3xl px-5 py-8 sm:px-8 sm:py-14">
        <h1 className="type-compact-page-title sm:type-page-title">{template.title()}</h1>
        <p className="mt-3 text-15 text-muted-foreground">{template.description()}</p>
        {available && owner ? (
          <section className="mt-6 rounded-xl border border-hairline bg-card p-5 sm:p-6" aria-label={t("New private chat with NyxBot")}>
            <div className="flex flex-wrap items-baseline justify-between gap-2">
              <h2 className="type-card-title">{t("Starter message")}</h2>
              {verified ? <p className="text-13 text-muted-foreground">{t("Verified Ornn skill · v{{version}}", { version: template.version })}</p> : null}
            </div>
            <p className="mt-3 text-15/relaxed break-words">{starter}</p>
            <p className="mt-5 text-14 text-muted-foreground">{t("Continuing adds this skill to your personal NyxBot and opens a private chat with a starter message you can edit before sending.")}</p>
            {issue ? <p role="alert" className="mt-3 text-14 text-destructive">{messages[issue]}</p> : null}
            {preview.isPending || starting ? <p role="status" className="mt-3 text-14 text-muted-foreground">{starting ? t("Opening your NyxBot chat…") : t("Verifying the published skill…")}</p> : null}
            <div className="mt-5 flex flex-wrap gap-2">
              {issue === "preview_temporary" ? (
                <Button onClick={() => { setFailure(undefined); void preview.refetch(); }} isLoading={preview.isFetching}>{t("Retry verification")}</Button>
              ) : issue === "preview_access" || issue === "preview_mismatch" ? (
                <Button asChild><Link to="/testing">{t("Choose another template")}</Link></Button>
              ) : (
                <Button variant="primary" onClick={() => void start()} isLoading={starting} disabled={!verified || starting}>
                  {failure ? t("Try again") : t("Open NyxBot")}
                  <ArrowRight aria-hidden="true" />
                </Button>
              )}
              {manageSkills ? (
                <Button asChild><Link to="/assistant" search={{ agent: skillsAgentId, details: "skills" }}>{t("Manage NyxBot skills")}</Link></Button>
              ) : null}
            </div>
          </section>
        ) : (
          <p role="status" className="mt-6 rounded-xl border border-hairline bg-card p-5 text-15 text-muted-foreground">{t("This template requires NyxBot, which is unavailable for this account.")}</p>
        )}
      </main>
    </div>
  );
}
