import { Link, useNavigate, useSearch } from "@tanstack/react-router";
import { ArrowRight, Check } from "lucide-react";
import { NyxidLogo } from "@/components/brand/nyxid-logo";
import { PageHeader } from "@/components/shared/page-header";
import { Button } from "@/components/ui/button";
import { LanguageSwitcher } from "@/features/landing/components/language-switcher";
import { useApplyTheme } from "@/hooks/use-theme";
import { useUiTranslation } from "@/i18n/use-ui-translation";
import { createSkillSetup } from "@/lib/assistant/skill-template-handoff";
import { findSkillTemplate } from "@/lib/assistant/skill-templates";
import { cn } from "@/lib/utils";
import { useAuthStore } from "@/stores/auth-store";

export function SkillTemplateWelcomePage() {
  const { t } = useUiTranslation();
  useApplyTheme();
  const navigate = useNavigate();
  const { skill, code } = useSearch({ from: "/testing/welcome" });
  const template = findSkillTemplate(skill);
  const signedIn = useAuthStore((state) => state.isAuthenticated);
  const steps = [t("Choose a template"), t("Log in or create an account"), t("Open NyxBot")];
  const currentStep = signedIn ? 2 : 1;

  function continueWith(destination: "/login" | "/register" | "/assistant") {
    if (!template) return;
    const auth = useAuthStore.getState();
    const owner = auth.isAuthenticated ? auth.user?.id : undefined;
    const search = createSkillSetup(template.slug, owner);
    if (owner || destination === "/assistant") {
      void navigate({ to: "/assistant", search });
      return;
    }
    const returnTo = new URL("/assistant", window.location.origin);
    returnTo.search = new URLSearchParams({ ...search }).toString();
    void navigate({ to: destination, search: { return_to: returnTo.href, code } });
  }

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
      <main className="mx-auto max-w-3xl px-5 py-8 sm:px-8 sm:py-14">
        {template ? (
          <>
            <PageHeader titleAs="h1" title={t("Awesome, let’s get you started.")} />
            <ol aria-label={t("Setup progress")} className="my-6 grid gap-3 sm:grid-cols-3">
              {steps.map((label, index) => (
                <li key={label} aria-current={index === currentStep ? "step" : undefined} className={cn(
                  "flex items-center gap-2 rounded-lg border border-hairline px-3 py-2 type-row-title text-muted-foreground",
                  index === currentStep && "border-primary bg-overlay text-foreground",
                )}>
                  <span aria-hidden="true" className="grid size-6 shrink-0 place-items-center rounded-full bg-overlay-strong type-secondary-strong">
                    {index < currentStep ? <Check className="size-3" /> : index + 1}
                  </span>
                  {label}
                  {index < currentStep ? <span className="sr-only">{t("Completed")}</span> : null}
                </li>
              ))}
            </ol>
            <section className="rounded-xl border border-hairline bg-card p-5 sm:p-6" aria-label={t("Selected template")}>
              <p className="type-secondary-strong text-muted-foreground">{t("Selected template")}</p>
              <h2 className="mt-1 type-card-title">{template.title()}</h2>
              <p className="mt-2 text-15 text-muted-foreground">{template.description()}</p>
              <p className="mt-4 text-14 text-muted-foreground">{t("Continuing adds this skill to your personal NyxBot and opens a private chat with a starter message you can edit before sending.")}</p>
              <div className="mt-5 flex flex-wrap gap-2">
                {signedIn ? (
                  <Button type="button" variant="primary" onClick={() => continueWith("/assistant")}>
                    {t("Open NyxBot")} <ArrowRight aria-hidden="true" />
                  </Button>
                ) : (
                  <>
                    <Button type="button" variant="primary" onClick={() => continueWith("/login")}>
                      {t("Log in")} <ArrowRight aria-hidden="true" />
                    </Button>
                    <Button type="button" onClick={() => continueWith("/register")}>
                      {t("Create an account")}
                    </Button>
                  </>
                )}
              </div>
            </section>
          </>
        ) : (
          <PageHeader titleAs="h1" title={t("Choose a template")} description={t("Choose a skill from the gallery to get started with NyxBot.")} />
        )}
        <Button asChild variant="ghost" className="mt-4">
          <Link to="/testing" search={{ skill, code }}>{t("Back to templates")}</Link>
        </Button>
      </main>
    </div>
  );
}
