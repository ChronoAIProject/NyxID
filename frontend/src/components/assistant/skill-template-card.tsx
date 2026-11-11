import { STATUS_CARD_MAX_W } from "@/lib/assistant/banner";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { useUiTranslation } from "@/i18n/use-ui-translation";

/** One step of a NYX template outline, as the backend's outline reader returns it. */
export interface SkillTemplateStep {
  readonly id: string;
  readonly title: string;
  readonly text: string;
  readonly uses?: { readonly name: string; readonly version: string } | null;
  readonly asks_you?: boolean;
}

/** A parsed NYX template outline (`docs/SKILL_TEMPLATES.md`). */
export interface SkillTemplateOutline {
  readonly overview: string;
  readonly steps: readonly SkillTemplateStep[];
  readonly needs?: string | null;
  readonly produces?: string | null;
}

export interface SkillTemplateCardProps {
  readonly name: string;
  readonly version: string;
  readonly kind: "skill" | "skillset";
  /** Skillset members, by name. */
  readonly members?: readonly string[];
  /** `null` when the skill has no valid outline. */
  readonly outline: SkillTemplateOutline | null;
  /** The skill is not attached yet, so starting attaches it first. */
  readonly needsAttach?: boolean;
  /** The owner already started this skill from the card. */
  readonly started?: boolean;
  readonly onStart?: () => void;
  readonly onDismiss?: () => void;
}

/** Overview paragraphs: blank lines separate paragraphs, single line breaks are wrapping. */
function paragraphs(text: string): string[] {
  return text
    .split(/\n\s*\n/)
    .map((paragraph) => paragraph.replace(/\s*\n\s*/g, " ").trim())
    .filter(Boolean);
}

/**
 * A skill card: the skill's own outline (never reworded), so the owner can
 * decide whether to start it. All text comes from the skill package and is
 * rendered as plain text. Starting is a choice, not an approval: actions in
 * the run still go through their own confirmations.
 */
export function SkillTemplateCard({
  name,
  version,
  kind,
  members = [],
  outline,
  needsAttach = false,
  started = false,
  onStart,
  onDismiss,
}: SkillTemplateCardProps) {
  const { t } = useUiTranslation();
  const label =
    kind === "skillset"
      ? t("Skillset · {{count}} skills", { count: members.length })
      : t("Skill");
  return (
    <section
      aria-label={kind === "skillset" ? t("{{name}} skillset", { name }) : t("{{name}} skill", { name })}
      className={`${STATUS_CARD_MAX_W} space-y-3 rounded-xl border border-border/50 bg-card p-4`}
    >
      <div className="space-y-1">
        <p className="type-group-label uppercase tracking-[1.5px] text-text-tertiary">{label}</p>
        <p className="type-card-title break-words text-foreground">
          {name} <span className="font-mono font-normal text-text-tertiary">{version}</span>
        </p>
      </div>

      {outline ? (
        <>
          <div className="space-y-2">
            {paragraphs(outline.overview).map((paragraph, index) => (
              <p key={index} className="type-body break-words text-muted-foreground">
                {paragraph}
              </p>
            ))}
          </div>

          <ol aria-label={t("Steps")} className="space-y-2.5">
            {outline.steps.map((step, index) => (
              <li key={step.id} className="flex gap-2.5">
                <span
                  aria-hidden
                  className="type-secondary-emphasis mt-px flex size-5 shrink-0 items-center justify-center rounded-full border border-border text-text-tertiary"
                >
                  {index + 1}
                </span>
                <div className="min-w-0 flex-1 space-y-1">
                  <p className="break-words">
                    <span className="type-row-title text-foreground">{step.title}. </span>
                    <span className="type-body text-muted-foreground">{step.text}</span>
                  </p>
                  {step.uses || step.asks_you ? (
                    <div className="flex flex-wrap gap-1.5">
                      {step.uses ? (
                        <Badge variant="secondary" className="max-w-full break-all">
                          {t("Uses {{name}}", { name: step.uses.name })}
                          <span className="ml-1 font-mono font-normal text-text-tertiary">{step.uses.version}</span>
                        </Badge>
                      ) : null}
                      {step.asks_you ? <Badge variant="info">{t("Asks you")}</Badge> : null}
                    </div>
                  ) : null}
                </div>
              </li>
            ))}
          </ol>

          {outline.needs || outline.produces ? (
            <dl className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-1 border-t border-border/50 pt-3">
              {outline.needs ? (
                <>
                  <dt className="type-group-label text-text-tertiary">{t("Needs")}</dt>
                  <dd className="type-secondary break-words text-muted-foreground">{outline.needs}</dd>
                </>
              ) : null}
              {outline.produces ? (
                <>
                  <dt className="type-group-label text-text-tertiary">{t("Produces")}</dt>
                  <dd className="type-secondary break-words text-muted-foreground">{outline.produces}</dd>
                </>
              ) : null}
            </dl>
          ) : null}
        </>
      ) : (
        <p className="type-secondary rounded-lg border border-dashed border-border p-3 text-text-tertiary">
          {t("This skill has no NYX template outline. NyxBot explains it from its guidance instead.")}
        </p>
      )}

      {kind === "skillset" && members.length ? (
        <div className="space-y-1.5 border-t border-border/50 pt-3">
          <p className="type-group-label text-text-tertiary">{t("Included skills")}</p>
          <div className="flex flex-wrap gap-1.5">
            {members.map((member) => (
              <Badge key={member} variant="secondary" className="max-w-full break-all">
                {member}
              </Badge>
            ))}
          </div>
        </div>
      ) : null}

      {started ? (
        <p role="status" className="type-secondary text-right text-text-tertiary">
          {t("Started. Progress continues below.")}
        </p>
      ) : (
        <div className="flex justify-end gap-2">
          <Button variant="outline" disabled={!onDismiss} onClick={onDismiss}>
            {t("Not now")}
          </Button>
          <Button variant="primary" disabled={!onStart || !outline} onClick={onStart}>
            {needsAttach ? t("Add & start") : t("Start")}
          </Button>
        </div>
      )}
    </section>
  );
}
