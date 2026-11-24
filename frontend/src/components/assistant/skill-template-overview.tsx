import { Badge } from "@/components/ui/badge";
import { useSkillPreview } from "@/hooks/use-agent-skills";
import { useUiTranslation } from "@/i18n/use-ui-translation";
import type { SkillTemplate } from "@/lib/assistant/skill-templates";
import type { AgentSkill } from "@/schemas/agent-skills";
import { SkillTemplateCard } from "./skill-template-card";

function isVerifiedTemplate(reference: AgentSkill | undefined, template: SkillTemplate) {
  return Boolean(reference && reference.source === "ornn" && !reference.disabled && reference.skill_id === template.skillId &&
    reference.version === template.version && reference.sha256 === template.sha256);
}

export function SkillTemplateOverview({ template, verify = true }: {
  readonly template: SkillTemplate;
  readonly verify?: boolean;
}) {
  const { t } = useUiTranslation();
  const preview = useSkillPreview(verify ? template.skillId : undefined, template.version);
  const details = !preview.isError && isVerifiedTemplate(preview.data?.reference, template) ? preview.data : undefined;
  if (details?.outline) {
    return <SkillTemplateCard name={template.title()} version={template.version} kind="skill"
      outline={details.outline} author={details.author} actions={false} />;
  }
  return (
    <section aria-label={t("Skill overview")} className="max-w-2xl space-y-3 rounded-xl border border-hairline bg-card p-4 sm:p-5">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <h2 className="type-card-title">{template.title()}</h2>
        <Badge variant="secondary">{t("Ornn skill · v{{version}}", { version: template.version })}</Badge>
      </div>
      <p className="type-body text-muted-foreground">{template.description()}</p>
      <div className="space-y-2">
        <h3 className="type-row-title">{t("Starting template")}</h3>
        <dl className="divide-y divide-hairline rounded-lg border border-hairline">
          {template.inputs().map(([label, hint]) => (
            <div key={label} className="grid gap-x-4 gap-y-0.5 px-3 py-2 sm:grid-cols-[minmax(8rem,1fr)_3fr]">
              <dt className="type-row-title">{label}</dt>
              <dd className="type-body text-muted-foreground">{hint}</dd>
            </div>
          ))}
        </dl>
      </div>
      <p className="type-body text-muted-foreground">
        <span className="mr-2 type-row-title text-foreground">{t("What you’ll get")}</span>{template.output()}
      </p>
      {details?.author?.name && <p className="type-secondary text-muted-foreground">{t("By {{name}}", { name: details.author.name })}</p>}
    </section>
  );
}
