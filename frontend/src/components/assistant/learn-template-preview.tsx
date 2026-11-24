import { useNavigate } from "@tanstack/react-router";
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
import { createSkillSetup } from "@/lib/assistant/skill-template-handoff";
import { formatTemplateStarter, type SkillTemplate } from "@/lib/assistant/skill-templates";
import { useAuthStore } from "@/stores/auth-store";

declare module "@tanstack/react-router" {
  interface HistoryState {
    skillTemplateLearnOrigin?: true;
  }
}

const focusRing = "focus-visible:ring-2 focus-visible:ring-input-focus focus-visible:ring-offset-2 focus-visible:ring-offset-background";

export function LearnTemplatePreview({ template, featured = false, available }: {
  readonly template: SkillTemplate;
  readonly featured?: boolean;
  readonly available: boolean;
}) {
  const { t } = useUiTranslation();
  const navigate = useNavigate();
  const owner = useAuthStore((state) => state.user?.id);
  const title = template.title();

  function confirm() {
    if (!owner || owner !== useAuthStore.getState().user?.id) return;
    void navigate({
      to: "/assistant",
      search: createSkillSetup(template.slug, owner),
      state: { skillTemplateLearnOrigin: true },
    });
  }

  return (
    <Dialog>
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
          <p className="mt-2 break-words text-15 text-muted-foreground">{formatTemplateStarter(template)}</p>
        </div>
        {available ? (
          <p className="text-14 text-muted-foreground">
            {t("Continuing adds this skill to your personal NyxBot and opens a private chat with a starter message you can edit before sending.")}{" "}
            {t("Once added, the skill stays in your NyxBot’s Skills list, even if you leave the chat.")}
          </p>
        ) : <p role="status" className="text-14 text-muted-foreground">{t("This template requires NyxBot, which is unavailable for this account.")}</p>}
        <DialogFooter>
          <DialogClose asChild><Button type="button" variant="outline" className={focusRing}>{t("Cancel")}</Button></DialogClose>
          {available && (
            <Button type="button" variant="primary" className={focusRing} onClick={confirm} disabled={!owner}>
              {t("Open NyxBot")} <ArrowRight aria-hidden="true" />
            </Button>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
