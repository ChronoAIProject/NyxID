import { Copy, ExternalLink, ShieldCheck, CircleHelp } from "lucide-react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { useChannelBot } from "@/hooks/use-channel-bots";
import {
  OnboardingShell,
  OnboardingNotice,
  StepHelp,
  BackButton,
} from "./onboarding-shell";

export function LinkChannelStep({
  botId,
  onBack,
}: {
  readonly botId: string;
  readonly onBack: (missing?: boolean) => void;
}) {
  const { t } = useTranslation();
  const bot = useChannelBot(botId);
  const channelName =
    bot.data?.platform === "whatsapp" ? "WhatsApp" : "Telegram";
  const ready =
    bot.data?.status === "active" &&
    bot.data.is_active &&
    bot.data.webhook_registered;
  return (
    <OnboardingShell
      title={t("linkTitle")}
      subtitle={t("linkSubtitle")}
      step={3}
      actions={
        <>
          <Button className="nb-primary" disabled>
            <ExternalLink size={14} />
            {t("openChat", { channel: channelName })}
          </Button>
          <Button className="nb-secondary" disabled>
            <Copy size={14} />
            {t("copyCode")}
          </Button>
        </>
      }
    >
      <p className="nb-next">{t("linkNext", { channel: channelName })}</p>
      <p className="nb-intro">
        {t("linkIntro")} <StepHelp>{t("linkHelp")}</StepHelp>
      </p>
      <ul className="nb-link-reasons">
        <li>
          <ShieldCheck size={14} />
          {t("linkReason")}
        </li>
        <li>
          <CircleHelp size={14} />
          {t("linkThen")}
        </li>
      </ul>
      <label className="nb-code-label" htmlFor="nyxbot-pairing-code">
        {t("code")}
      </label>
      <Input id="nyxbot-pairing-code" placeholder={t("codePending")} disabled />
      {bot.isPending ? (
        <OnboardingNotice>{t("loading")}</OnboardingNotice>
      ) : bot.isError ? (
        <OnboardingNotice error>
          {t("loadError")}{" "}
          <Button onClick={() => void bot.refetch()}>{t("retry")}</Button>
        </OnboardingNotice>
      ) : (
        <OnboardingNotice>
          {t(ready ? "linkUnavailable" : "channelPending")}
        </OnboardingNotice>
      )}
      <div className="nb-inline-actions">
        {bot.data && (
          <Button className="nb-secondary" asChild>
            <a href={`/channel-bots/${encodeURIComponent(botId)}`}>
              <ExternalLink size={14} />
              {t("manageChannel")}
            </a>
          </Button>
        )}
        <BackButton
          onClick={() =>
            onBack(
              Boolean(
                bot.error && "status" in bot.error && bot.error.status === 404,
              ),
            )
          }
        />
      </div>
    </OnboardingShell>
  );
}
