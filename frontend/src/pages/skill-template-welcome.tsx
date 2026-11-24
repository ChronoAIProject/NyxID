import { useRef } from "react";
import { useNavigate, useRouter, useRouterState, useSearch } from "@tanstack/react-router";
import { ArrowRight, Check } from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogClose,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { useUiTranslation } from "@/i18n/use-ui-translation";
import { createSkillSetup } from "@/lib/assistant/skill-template-handoff";
import { findSkillTemplate } from "@/lib/assistant/skill-templates";
import { cn } from "@/lib/utils";
import { useAuthStore } from "@/stores/auth-store";

declare module "@tanstack/react-router" {
  interface HistoryState {
    skillTemplateGalleryOrigin?: true;
  }
}

export function SkillTemplateWelcomePage() {
  const { t } = useUiTranslation();
  const router = useRouter();
  const navigate = useNavigate();
  const fromGallery = useRouterState({ select: (state) => state.location.state.skillTemplateGalleryOrigin === true });
  const { skill, code } = useSearch({ from: "/testing/welcome" });
  const template = findSkillTemplate(skill);
  const signedIn = useAuthStore((state) => state.isAuthenticated);
  const title = useRef<HTMLHeadingElement>(null);
  const steps = [t("Choose a template"), t("Log in or create an account"), t("Open NyxBot")];
  const currentStep = signedIn ? 2 : 1;

  function close() {
    if (fromGallery) router.history.back();
    else void navigate({ to: "/testing", search: { skill, code }, replace: true });
  }

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
    <Dialog open onOpenChange={(open) => { if (!open) close(); }}>
      <DialogContent
        className="md:max-w-xl"
        onOpenAutoFocus={(event) => {
          event.preventDefault();
          title.current?.focus();
        }}
        onCloseAutoFocus={(event) => {
          event.preventDefault();
          if (router.state.location.pathname !== "/testing") return;
          const opener = template && document.querySelector<HTMLElement>(`[data-try-template="${template.slug}"]`);
          (opener || document.querySelector<HTMLElement>("[data-try-template]"))?.focus();
        }}
      >
        <DialogHeader className="pr-8">
          <DialogTitle ref={title} tabIndex={-1}>
            {template ? t("Awesome, let’s get you started.") : t("Choose a template")}
          </DialogTitle>
          <DialogDescription>
            {template
              ? t("Continuing adds this skill to your personal NyxBot and opens a private chat with a starter message you can edit before sending.")
              : t("Choose a skill from the gallery to get started with NyxBot.")}
          </DialogDescription>
        </DialogHeader>
        {template ? (
          <>
            <ol aria-label={t("Setup progress")} className="grid gap-2 sm:grid-cols-3">
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
            <section className="rounded-xl border border-hairline bg-card p-4" aria-label={t("Selected template")}>
              <p className="type-secondary-strong text-muted-foreground">{t("Selected template")}</p>
              <h3 className="mt-1 type-card-title">{template.title()}</h3>
              <p className="mt-2 text-15 text-muted-foreground">{template.description()}</p>
            </section>
            <DialogFooter>
              {signedIn ? (
                <Button type="button" variant="primary" onClick={() => continueWith("/assistant")}>
                  {t("Open NyxBot")} <ArrowRight aria-hidden="true" />
                </Button>
              ) : (
                <>
                  <Button type="button" variant="outline" onClick={() => continueWith("/register")}>
                    {t("Create an account")}
                  </Button>
                  <Button type="button" variant="primary" onClick={() => continueWith("/login")}>
                    {t("Log in")} <ArrowRight aria-hidden="true" />
                  </Button>
                </>
              )}
            </DialogFooter>
          </>
        ) : (
          <DialogFooter>
            <DialogClose asChild><Button type="button">{t("Back to templates")}</Button></DialogClose>
          </DialogFooter>
        )}
      </DialogContent>
    </Dialog>
  );
}
