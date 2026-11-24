import { Link, Outlet, useSearch } from "@tanstack/react-router";
import { ArrowRight, BookOpenCheck, ClipboardCheck, Megaphone, PieChart } from "lucide-react";
import { NyxidLogo } from "@/components/brand/nyxid-logo";
import { Button } from "@/components/ui/button";
import { LanguageSwitcher } from "@/features/landing/components/language-switcher";
import { useApplyTheme } from "@/hooks/use-theme";
import { useUiTranslation } from "@/i18n/use-ui-translation";
import { findSkillTemplate, SKILL_TEMPLATES } from "@/lib/assistant/skill-templates";

export function TestingPage() {
  const { t } = useUiTranslation();
  useApplyTheme();
  const { skill, code } = useSearch({ from: "/testing" });
  const selected = findSkillTemplate(skill);
  const templates = selected
    ? [selected, ...SKILL_TEMPLATES.filter((template) => template !== selected)]
    : SKILL_TEMPLATES;
  const categories = {
    research: { label: t("Research"), icon: BookOpenCheck },
    communications: { label: t("Communications"), icon: Megaphone },
    operations: { label: t("Operations"), icon: PieChart },
    engineering: { label: t("Engineering"), icon: ClipboardCheck },
  };

  return (
    <div className="min-h-dvh bg-background text-foreground">
      <header className="border-b border-hairline">
        <div className="mx-auto flex max-w-6xl items-center justify-between gap-4 px-5 py-4 sm:px-8">
          <Link to="/" className="rounded focus-visible:outline-2 focus-visible:outline-offset-4 focus-visible:outline-primary">
            <NyxidLogo />
          </Link>
          <LanguageSwitcher />
        </div>
      </header>
      <main className="mx-auto max-w-6xl px-5 pb-16 sm:px-8">
        <div className="border-b border-hairline py-10 sm:py-14">
          <h1 className="type-compact-page-title sm:type-page-title">{t("NyxBot skill templates")}</h1>
          <p className="mt-3 max-w-xl text-16 text-muted-foreground">
            {t("Pick a task to start its skill in a private NyxBot chat.")}
          </p>
        </div>
        <ul className="mt-6 grid grid-cols-1 gap-4 sm:grid-cols-2 lg:grid-cols-3">
          {templates.map((template, index) => {
            const { label, icon: Icon } = categories[template.category];
            const title = template.title();
            const search = { skill: template.slug };
            const actionName = t("Try this · {{title}}", { title });
            return (
              <li key={template.slug} className={index === 0 ? "sm:col-span-full" : undefined}>
                <article className="flex h-full flex-col items-start gap-4 rounded-xl border border-hairline bg-card p-5 sm:p-6">
                  <div className="flex items-center gap-2 type-secondary-strong text-muted-foreground">
                    <span className="grid size-8 place-items-center rounded-lg bg-overlay text-primary-text"><Icon aria-hidden="true" className="size-4" /></span>
                    {label}
                  </div>
                  <div className="flex-1">
                    <h2 className="type-card-title text-foreground">
                      <Link to="/testing" search={{ ...search, code }} aria-current={skill === template.slug ? "true" : undefined} className="rounded underline-offset-4 hover:underline focus-visible:outline-2 focus-visible:outline-offset-4 focus-visible:outline-primary">
                        {title}
                      </Link>
                    </h2>
                    <p className="mt-2 max-w-xl text-15 text-muted-foreground">{template.description()}</p>
                  </div>
                  <Button asChild variant={index === 0 ? "primary" : "default"}>
                    <Link to="/testing/welcome" search={{ ...search, code }} state={{ skillTemplateGalleryOrigin: true }} data-try-template={template.slug} aria-label={actionName}>
                      {t("Try this")} <ArrowRight aria-hidden="true" />
                    </Link>
                  </Button>
                </article>
              </li>
            );
          })}
        </ul>
      </main>
      <Outlet />
    </div>
  );
}
