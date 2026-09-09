import { useState } from "react";
import { zodResolver } from "@hookform/resolvers/zod";
import { Eye, EyeOff, ExternalLink } from "lucide-react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  useAppForm,
  Form,
  FormField,
  FormItem,
  FormLabel,
  FormControl,
  FormMessage,
} from "@/components/ui/form";
import { useCreateChannelBot } from "@/hooks/use-channel-bots";
import { useManagedOnboarding } from "@/hooks/use-channel-managed";
import {
  nyxbotTelegramSchema,
  type NyxbotTelegramForm,
  type NyxbotChannel,
} from "@/schemas/nyxbot-onboarding";
import type { CreateChannelBotResponse } from "@/types/channels";
import {
  BackButton,
  OnboardingNotice,
  OnboardingShell,
  StepHelp,
} from "./onboarding-shell";
import { BrandIcon } from "./brand-icon";

const CHANNEL_AVAILABILITY: Record<NyxbotChannel, boolean> = {
  telegram: true,
  whatsapp: false,
};

export function ChannelStep({
  channel: preferredChannel,
  referral,
  onSelect,
  onBack,
  onConnected,
  onSpendingCap,
}: {
  readonly channel: NyxbotChannel | null;
  readonly referral?: NyxbotChannel;
  readonly onSelect: (channel: NyxbotChannel) => void;
  readonly onBack: () => void;
  readonly onConnected: (bot: CreateChannelBotResponse) => void;
  readonly onSpendingCap: () => void;
}) {
  const { t } = useTranslation();
  const channel =
    preferredChannel && CHANNEL_AVAILABILITY[preferredChannel]
      ? preferredChannel
      : null;
  const [showToken, setShowToken] = useState(false);
  const createBot = useCreateChannelBot();
  const managed = useManagedOnboarding("whatsapp", channel === "whatsapp");
  const form = useAppForm<NyxbotTelegramForm>({
    resolver: zodResolver(nyxbotTelegramSchema),
    defaultValues: { bot_token: "" },
    mode: "onChange",
  });
  async function submit(values: NyxbotTelegramForm) {
    try {
      const result = await createBot.mutateAsync({
        platform: "telegram",
        label: "Nyxbot Telegram",
        bot_token: values.bot_token.trim(),
      });
      form.reset();
      createBot.reset();
      onConnected(result);
    } catch {
      // Do not echo provider errors that might contain submitted credentials.
      form.setError("bot_token", { message: t("channelFailed") });
    }
  }
  return (
    <OnboardingShell
      title={t("channelTitle")}
      step={3}
      actions={
        <>
          {channel === "whatsapp" ? (
            <Button
              type="button"
              className="nb-primary"
              onClick={onSpendingCap}
            >
              {t("reviewCap")}
            </Button>
          ) : (
            <Button
              form="nyxbot-telegram"
              type="submit"
              className="nb-primary"
              isLoading={createBot.isPending}
              disabled={channel !== "telegram" || !form.formState.isValid}
            >
              {t("connectChannel")}
            </Button>
          )}
          <BackButton onClick={onBack} disabled={createBot.isPending} />
        </>
      }
    >
      <p className="nb-intro">
        {t("channelIntro")} <StepHelp>{t("channelHelp")}</StepHelp>
      </p>
      <fieldset
        className="nb-tiles nb-channel-tiles"
        disabled={createBot.isPending}
      >
        <legend className="sr-only">{t("channelTitle")}</legend>
        {(["telegram", "whatsapp"] as const).map((option) => (
          <label
            className={
              CHANNEL_AVAILABILITY[option]
                ? "nb-tile"
                : "nb-tile nb-tile-unavailable"
            }
            data-selected={channel === option}
            key={option}
          >
            <input
              type="radio"
              name="channel"
              value={option}
              disabled={!CHANNEL_AVAILABILITY[option]}
              checked={channel === option}
              onChange={() => {
                form.reset();
                setShowToken(false);
                onSelect(option);
              }}
            />
            <BrandIcon brand={option} />
            <strong>{t(option)}</strong>
            <span>
              {t(
                !CHANNEL_AVAILABILITY[option]
                  ? "comingSoon"
                  : channel === option
                    ? referral === option
                      ? "preselected"
                      : "selected"
                    : "available",
              )}
            </span>
          </label>
        ))}
        {[
          { id: "instagram", name: "Instagram" },
          { id: "facebook", name: "FB Messenger" },
        ].map((option) => (
          <div
            className="nb-tile nb-tile-unavailable"
            aria-disabled="true"
            key={option.id}
          >
            <BrandIcon brand={option.id} />
            <strong>{option.name}</strong>
            <span>{t("comingSoon")}</span>
          </div>
        ))}
      </fieldset>
      {channel === "telegram" && (
        <>
          <ol className="nb-instructions">
            <li>
              <a
                href="https://t.me/BotFather"
                target="_blank"
                rel="noopener noreferrer"
              >
                {t("telegramOpen")} <ExternalLink size={12} />
              </a>{" "}
              {t("telegramCommand")}
            </li>
            <li>{t("telegramName")}</li>
            <li>{t("telegramPaste")}</li>
          </ol>
          <Form {...form}>
            <form id="nyxbot-telegram" onSubmit={form.handleSubmit(submit)}>
              <FormField
                control={form.control}
                name="bot_token"
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>{t("token")}</FormLabel>
                    <div className="nb-token-field">
                      <FormControl>
                        <Input
                          {...field}
                          disabled={createBot.isPending}
                          type={showToken ? "text" : "password"}
                          placeholder={t("tokenPlaceholder")}
                          autoComplete="off"
                          spellCheck={false}
                          autoCapitalize="none"
                        />
                      </FormControl>
                      <button
                        type="button"
                        disabled={createBot.isPending}
                        aria-label={t(showToken ? "hideToken" : "showToken")}
                        title={t(showToken ? "hideToken" : "showToken")}
                        onClick={() => setShowToken((shown) => !shown)}
                      >
                        {showToken ? <EyeOff size={16} /> : <Eye size={16} />}
                      </button>
                    </div>
                    <FormMessage />
                  </FormItem>
                )}
              />
            </form>
          </Form>
          <p className="nb-small">{t("tokenNote")}</p>
        </>
      )}
      {channel === "whatsapp" && (
        <>
          <p className="nb-intro">
            {t("whatsappIntro")} <StepHelp>{t("whatsappHelp")}</StepHelp>
          </p>
          <ol className="nb-instructions">
            <li>{t("whatsappLogin")}</li>
            <li>{t("whatsappNumber")}</li>
            <li>{t("whatsappVerify")}</li>
          </ol>
          <OnboardingNotice>{t("capUnavailable")}</OnboardingNotice>
          {managed.isPending && (
            <p role="status" className="nb-small">
              {t("loading")}
            </p>
          )}
          {(managed.isError || (managed.data && !managed.data.available)) && (
            <OnboardingNotice>{t("whatsappUnavailable")}</OnboardingNotice>
          )}
          <Button type="button" className="nb-secondary nb-full" disabled>
            <ExternalLink size={14} />
            {t("connectWhatsapp")}
          </Button>
        </>
      )}
    </OnboardingShell>
  );
}
