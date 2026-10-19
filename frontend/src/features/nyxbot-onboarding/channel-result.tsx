import { ArrowLeft, ExternalLink } from "lucide-react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import { BrandIcon } from "./brand-icon";
import { OnboardingShell } from "./onboarding-shell";

export function ChannelResult({
  telegramUrl,
  onAddAnother,
}: {
  readonly telegramUrl: string;
  readonly onAddAnother: () => void;
}) {
  const { t } = useTranslation();
  const username = new URL(telegramUrl).pathname.slice(1);
  return (
    <OnboardingShell
      title={t("channelConnectedTitle")}
      subtitle={t("channelConnectedSubtitle")}
      step={3}
      complete
      variant="setup"
      actions={
        <>
          <Button className="nb-secondary" onClick={onAddAnother}>
            <ArrowLeft size={18} aria-hidden="true" />
            {t("connectAnotherBot")}
          </Button>
          <Button className="nb-primary" asChild>
            <a href={telegramUrl} target="_blank" rel="noopener noreferrer">
              <ExternalLink size={18} aria-hidden="true" />
              {t("openTelegram")}
            </a>
          </Button>
        </>
      }
    >
      <div className="nb-bot-result">
        <BrandIcon brand="telegram" />
        <strong>@{username}</strong>
      </div>
    </OnboardingShell>
  );
}
