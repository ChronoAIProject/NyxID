import { useState } from "react";
import { useNavigate, useSearch } from "@tanstack/react-router";
import { I18nextProvider, useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { usePublicUseCases } from "@/hooks/use-public-help";
import { nyxbotI18n } from "@/features/nyxbot-onboarding/i18n";
import {
  OnboardingNotice,
  OnboardingShell,
} from "@/features/nyxbot-onboarding/onboarding-shell";
import "@/features/nyxbot-onboarding/onboarding.css";

function StartContent() {
  const { t } = useTranslation();
  const search = useSearch({ from: "/nyxbot/start" });
  const navigate = useNavigate({ from: "/nyxbot/start" });
  const help = usePublicUseCases();
  const [selected, setSelected] = useState(
    () => search.usecases?.split(",") ?? [],
  );
  const ids = selected.filter((id) =>
    help.data?.some((item) => item.id === id),
  );
  return (
    <OnboardingShell
      title={t("starterTitle")}
      subtitle={t("starterDescription")}
      step={null}
      actions={
        <Button
          variant="primary"
          disabled={!ids.length || !help.isSuccess}
          onClick={() =>
            void navigate({
              to: "/assistant",
              search: { draft: true, starter: ids.join(",") },
            })
          }
        >
          {t("starterContinue")}
        </Button>
      }
    >
      {help.isPending && <OnboardingNotice>{t("loading")}</OnboardingNotice>}
      {help.isError && (
        <OnboardingNotice error>
          {t("starterLoadError")}{" "}
          <Button onClick={() => void help.refetch()}>{t("retry")}</Button>
        </OnboardingNotice>
      )}
      {help.isSuccess && !help.data.length && (
        <OnboardingNotice>{t("starterEmpty")}</OnboardingNotice>
      )}
      {help.isSuccess && (
        <div className="space-y-3">
          {help.data.map((item) => (
            <label
              key={item.id}
              className="flex cursor-pointer items-start gap-3 rounded-lg border border-border bg-card p-4 text-12"
            >
              <Checkbox
                checked={ids.includes(item.id)}
                disabled={ids.length >= 8 && !ids.includes(item.id)}
                onCheckedChange={(checked) =>
                  setSelected(
                    checked
                      ? [...ids, item.id]
                      : ids.filter((id) => id !== item.id),
                  )
                }
                aria-label={item.title}
              />
              <span>
                <span className="block font-display text-15 text-foreground">
                  {item.title}
                </span>
                <span className="mt-1 block text-muted-foreground">
                  {item.description}
                </span>
              </span>
            </label>
          ))}
        </div>
      )}
    </OnboardingShell>
  );
}

export function NyxbotStartPage() {
  return (
    <I18nextProvider i18n={nyxbotI18n}>
      <StartContent />
    </I18nextProvider>
  );
}
