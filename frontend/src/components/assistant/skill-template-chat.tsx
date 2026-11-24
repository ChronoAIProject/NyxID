import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { Link, useNavigate, useRouter } from "@tanstack/react-router";
import { ArrowLeft, ArrowRight } from "lucide-react";
import { z } from "zod";
import { Button } from "@/components/ui/button";
import { useApplyTheme } from "@/hooks/use-theme";
import { useUiTranslation } from "@/i18n/use-ui-translation";
import { ApiError } from "@/lib/api-client";
import { assistantJson } from "@/lib/assistant/assistant-http";
import { getAssistantIdentityRevision } from "@/lib/assistant/identity";
import { parseAssistantSearch } from "@/lib/assistant/search";
import { claimSkillSetup, createSkillSetup, readTemplateLaunch, saveTemplateLaunch } from "@/lib/assistant/skill-template-handoff";
import type { SkillTemplate } from "@/lib/assistant/skill-templates";
import { useAuthStore } from "@/stores/auth-store";
import { SkillTemplateOverview } from "./skill-template-overview";

const launchResponse = z.object({ id: z.string().regex(/^nyxa-[0-9a-f]{32}$/) });

export function SkillTemplateChat({ template, available, setupId }: {
  readonly template: SkillTemplate;
  readonly available: boolean;
  readonly setupId?: string;
}) {
  const { t } = useUiTranslation();
  useApplyTheme("assistant");
  const router = useRouter();
  const navigate = useNavigate();
  const owner = useAuthStore((state) => state.user?.id);
  const [starting, setStarting] = useState(false);
  const [failure, setFailure] = useState<"access" | "mismatch" | "retry" | "conflict">();
  const mounted = useRef(false);
  const automaticChecked = useRef(false);
  const attempt = useRef(false);
  useLayoutEffect(() => {
    mounted.current = true;
    return () => { mounted.current = false; };
  }, []);

  async function start() {
    if (!available || !owner || attempt.current) return;
    attempt.current = true;
    const revision = getAssistantIdentityRevision();
    if (!setupId) {
      const search = createSkillSetup(template.slug, owner);
      const mock = parseAssistantSearch(router.state.location.search).mock;
      saveTemplateLaunch(search, owner);
      try {
        await navigate({ to: "/assistant", search: { ...search, mock }, replace: true });
      } catch {
        if (mounted.current) setFailure("retry");
      } finally {
        attempt.current = false;
      }
      return;
    }
    const id = setupId;
    const current = () => {
      const location = router.state.location;
      const search = parseAssistantSearch(location.search);
      return mounted.current && owner === useAuthStore.getState().user?.id &&
        revision === getAssistantIdentityRevision() && location.pathname === "/assistant" &&
        search.skill === template.slug && search.skillSetup === setupId;
    };
    const guard = () => {
      if (!current()) throw new DOMException("Template selection changed", "AbortError");
    };
    setStarting(true);
    setFailure(undefined);
    saveTemplateLaunch({ skill: template.slug, skillSetup: id }, owner);
    try {
      const result = launchResponse.parse(await assistantJson(
        `/assistant/nyxagent/skill-templates/${template.slug}/start`,
        { method: "POST", body: { setup_id: id }, authorityGuard: guard },
      ));
      guard();
      const mock = parseAssistantSearch(router.state.location.search).mock;
      await navigate({ to: "/assistant", search: { c: result.id, template: template.slug, mock }, replace: true });
    } catch (error) {
      if (!current()) return;
      if (error instanceof ApiError) {
        const details = error.errorResponse.details as { reason?: unknown } | undefined;
        if (details?.reason === "pinned_version_mismatch") setFailure("mismatch");
        else if (error.status === 409) setFailure("conflict");
        else if ([401, 403, 404].includes(error.status)) setFailure("access");
        else setFailure("retry");
      } else setFailure("retry");
    } finally {
      attempt.current = false;
      if (current()) setStarting(false);
    }
  }

  useEffect(() => {
    if (!available || !setupId || !owner || automaticChecked.current) return;
    automaticChecked.current = true;
    const setup = { skill: template.slug, skillSetup: setupId };
    if (claimSkillSetup(setup, owner) || readTemplateLaunch(setup, owner)) void start();
    // The first valid gallery intent owns the automatic launch for this mount.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [available, owner, setupId, template.slug]);

  const errors = {
    access: t("Your account can’t open this Ornn skill. Check your Ornn access or choose another template."),
    mismatch: t("NyxBot could not verify the published package against this template’s pinned version. Choose another template."),
    conflict: t("NyxBot couldn’t add this skill. Check its pinned version and the available space in Skills."),
    retry: t("NyxBot couldn’t start this skill. Try again."),
  };

  return (
    <div className="min-h-dvh bg-background text-foreground">
      <main className="mx-auto max-w-3xl space-y-4 px-5 py-8 sm:px-8 sm:py-12">
        <Button asChild variant="ghost">
          <Link to="/testing" search={{ skill: template.slug }}><ArrowLeft aria-hidden="true" />{t("Skill templates")}</Link>
        </Button>
        <h1 className="type-compact-page-title sm:type-page-title">{template.title()}</h1>
        <SkillTemplateOverview template={template} verify={false} />
        {failure && <p role="alert" className="type-body text-destructive">{errors[failure]}</p>}
        {starting && <p role="status" className="type-body text-muted-foreground">{t("Starting your skill in NyxBot…")}</p>}
        {available && owner ? (
          <div className="flex flex-wrap gap-2">
            {failure !== "mismatch" && failure !== "access" && (
              <Button variant="primary" onClick={() => void start()} isLoading={starting} disabled={starting}>
                {failure ? t("Try again") : t("Start skill")} <ArrowRight aria-hidden="true" />
              </Button>
            )}
            {failure === "conflict" && (
              <Button asChild><Link to="/assistant" search={{ details: "skills" }}>{t("Manage NyxBot skills")}</Link></Button>
            )}
          </div>
        ) : (
          <p role="status" className="type-body text-muted-foreground">{t("This template requires NyxBot, which is unavailable for this account.")}</p>
        )}
      </main>
    </div>
  );
}
