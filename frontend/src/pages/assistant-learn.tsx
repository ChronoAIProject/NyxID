import { Link, useNavigate } from "@tanstack/react-router";
import { ArrowDown, ArrowRight, BookOpen, Compass } from "lucide-react";
import { LearnTemplatePreview } from "@/components/assistant/learn-template-preview";
import { useIntroReplay } from "@/hooks/use-assistant-onboarding";
import { useFeature } from "@/hooks/use-feature-flag";
import { useUiTranslation } from "@/i18n/use-ui-translation";
import { findSkillTemplate, SKILL_TEMPLATES, type SkillTemplate } from "@/lib/assistant/skill-templates";
import { FEATURE_FLAG } from "@/lib/feature-flags";

const featured = findSkillTemplate("weekly-budget-variance-digest");
const categories = ["research", "communications", "operations", "engineering"] as const;
const groups = categories
  .map((category) => ({ category, templates: SKILL_TEMPLATES.filter((template) => template.category === category) }))
  .filter((group) => group.templates.length > 0);

const focusClass = "focus-visible:outline-solid focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-input-focus";
const resourceClass = `flex min-w-0 items-center gap-4 rounded-xl border border-hairline bg-card p-4 text-left transition-colors hover:border-hairline-strong ${focusClass}`;

function TemplateDetails({ template }: { readonly template: SkillTemplate }) {
  const { t } = useUiTranslation();
  return (
    <dl className="grid content-start gap-3 type-body">
      <div>
        <dt className="type-secondary-strong text-foreground">{t("What to supply")}</dt>
        <dd className="text-muted-foreground">{template.input()}</dd>
      </div>
      <div>
        <dt className="type-secondary-strong text-foreground">{t("What you’ll get")}</dt>
        <dd className="text-muted-foreground">{template.description()}</dd>
      </div>
    </dl>
  );
}

export function AssistantLearnPage() {
  const { t } = useUiTranslation();
  const navigate = useNavigate();
  const replayIntro = useIntroReplay((state) => state.replay);
  const nyxagentEnabled = useFeature(FEATURE_FLAG.NYXAGENT_ENGINE);
  const labels = {
    research: t("Research"),
    communications: t("Communications"),
    operations: t("Operations"),
    engineering: t("Engineering"),
  };

  return (
    <div className="mx-auto w-full max-w-5xl space-y-8 text-foreground">
      <h1 className="type-compact-page-title sm:type-page-title">{t("Learn")}</h1>
      {featured && (
        <section aria-labelledby="learn-featured" className="grid gap-5 rounded-2xl border border-hairline bg-card p-5 shadow-sm sm:grid-cols-2 sm:items-center sm:gap-8 sm:p-8">
          <div className="space-y-3">
            <h2 id="learn-featured" className="type-group-heading text-text-tertiary">{t("Featured template")}</h2>
            <h3 className="type-section-heading">{featured.title()}</h3>
            <LearnTemplatePreview available={nyxagentEnabled} template={featured} featured />
          </div>
          <div className="rounded-xl bg-overlay p-4">
            <TemplateDetails template={featured} />
          </div>
        </section>
      )}

      <section aria-labelledby="learn-getting-started" className="space-y-3">
        <h2 id="learn-getting-started" className="type-section-heading">{t("Getting started")}</h2>
        <div className="grid gap-3 sm:grid-cols-2">
          {nyxagentEnabled && (
            <button
              type="button"
              className={resourceClass}
              onClick={() => {
                replayIntro();
                void navigate({ to: "/assistant", search: {} });
              }}
            >
              <Compass aria-hidden="true" className="size-5 shrink-0 text-text-tertiary" />
              <span className="min-w-0 flex-1">
                <span className="block type-card-title">{t("Replay intro")}</span>
                <span className="mt-1 block type-body text-muted-foreground">{t("Replay the NyxBot intro to find recommended next steps and take the workspace tour.")}</span>
              </span>
              <ArrowRight aria-hidden="true" className="size-4 shrink-0 text-text-tertiary" />
            </button>
          )}
          <Link to="/docs" className={resourceClass}>
            <BookOpen aria-hidden="true" className="size-5 shrink-0 text-text-tertiary" />
            <span className="min-w-0 flex-1">
              <span className="block type-card-title">{t("Usage guides")}</span>
              <span className="mt-1 block type-body text-muted-foreground">{t("Find help setting up NyxBot and connecting services.")}</span>
            </span>
            <ArrowRight aria-hidden="true" className="size-4 shrink-0 text-text-tertiary" />
          </Link>
        </div>
      </section>

      <section aria-labelledby="learn-browse" className="space-y-6">
        <div className="space-y-3">
          <h2 id="learn-browse" className="type-section-heading">{t("Browse by task")}</h2>
          <nav aria-labelledby="learn-browse" className="flex flex-wrap gap-2">
            {groups.map(({ category }) => (
              <a key={category} href={`#learn-${category}`} className={`inline-flex min-h-control items-center gap-1.5 rounded-lg border border-hairline px-3 py-1.5 type-button hover:bg-overlay ${focusClass}`}>
                {labels[category]} <ArrowDown aria-hidden="true" className="size-3" />
              </a>
            ))}
          </nav>
        </div>
        {groups.map(({ category, templates }) => (
          <section key={category} aria-labelledby={`learn-${category}`} className="space-y-3">
            <h3 id={`learn-${category}`} tabIndex={-1} className={`w-fit scroll-mt-4 rounded type-card-title ${focusClass}`}>
              {labels[category]}
            </h3>
            <ul className="grid gap-3 sm:grid-cols-2">
              {templates.map((template) => (
                <li key={template.slug} className="flex min-w-0 flex-col items-start gap-4 rounded-xl border border-hairline bg-card p-5">
                  <h4 className="type-card-title">{template.title()}</h4>
                  <TemplateDetails template={template} />
                  <div className="mt-auto"><LearnTemplatePreview available={nyxagentEnabled} template={template} /></div>
                </li>
              ))}
            </ul>
          </section>
        ))}
      </section>
    </div>
  );
}
