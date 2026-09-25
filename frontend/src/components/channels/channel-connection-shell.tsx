import type { ReactNode } from "react";
import { ExternalLink } from "lucide-react";
import { NyxidIcon } from "@/components/brand/nyxid-icon";
import { ServiceIcon } from "@/components/service-icon";
import { TelegramGlyph } from "@/components/service-icons/_shared";
import { ConnectionArc } from "@/components/shared/connection-arc";
import { useApplyTheme } from "@/hooks/use-theme";

function ChannelIcon({ platform }: { readonly platform: string }) {
  if (platform === "telegram" || platform === "telegram-new") {
    return <TelegramGlyph className="size-9 text-foreground" />;
  }
  if (platform === "whatsapp") {
    return <img src="/nyxbot-whatsapp.svg" alt="" className="size-9 dark:invert" />;
  }
  return (
    <ServiceIcon
      slug={`api-${platform === "x" ? "twitter" : platform}`}
      size="xl"
      className="text-foreground"
    />
  );
}

export function ChannelConnectionShell({
  platform,
  platformName,
  complete,
  email,
  guideUrl,
  children,
}: {
  readonly platform: string;
  readonly platformName?: string;
  readonly complete: boolean;
  readonly email?: string;
  readonly guideUrl?: string | null;
  readonly children: ReactNode;
}) {
  useApplyTheme();
  return (
    <div className="flex min-h-dvh flex-col bg-background px-4 py-6 text-foreground sm:py-10">
      <div className="m-auto w-full max-w-[520px]">
        <main className="channel-connection-card relative overflow-hidden rounded-xl border border-border bg-card">
          <div className="px-5 py-8 sm:px-8">
            {guideUrl && (
              <div className="mb-6 flex justify-end">
                <a
                  href={guideUrl}
                  target="_blank"
                  rel="noopener noreferrer"
                  className="inline-flex items-center gap-1.5 text-xs text-muted-foreground hover:text-foreground"
                >
                  Channel setup guide
                  <ExternalLink className="size-3" aria-hidden="true" />
                </a>
              </div>
            )}
            <div
              className="mb-6 flex items-start justify-center"
              role="img"
              aria-label={`NyxID connects to ${platformName ?? "your channel"}`}
            >
              <div className="flex w-20 flex-col items-center gap-2.5">
                <div className="connection-identity-icon flex size-18 items-center justify-center overflow-hidden rounded-full border border-border bg-background">
                  <NyxidIcon className="size-9" alt="" />
                </div>
                <span className="text-[12px] font-medium">NyxID</span>
              </div>
              <ConnectionArc complete={complete} />
              <div className="flex w-20 flex-col items-center gap-2.5">
                <div className="connection-identity-icon flex size-18 items-center justify-center overflow-hidden rounded-full border border-border bg-background">
                  <ChannelIcon platform={platform} />
                </div>
                <span className="text-center text-[12px] font-medium">
                  {platformName ?? "Channel"}
                </span>
              </div>
            </div>
            <div className="mb-7 space-y-2 text-center">
              <h1 className="font-display text-[22px] font-medium leading-tight tracking-tight sm:text-[28px]">
                {platform === "telegram"
                  ? complete
                    ? "Your Telegram bot is connected"
                    : "Connect your Telegram bot"
                  : platformName
                    ? complete
                      ? `Your ${platformName} channel bot is created`
                      : `Create your ${platformName} channel bot`
                    : "Set up your channel bot"}
              </h1>
              <p className="text-[12px] leading-relaxed text-muted-foreground">
                {complete
                  ? "Finish any platform setup, then choose an agent to respond to your messages."
                  : platform === "telegram-new"
                    ? "Create a bot in Telegram and connect it to your AI agents through NyxID."
                    : `Connect your ${platformName ?? "channel"} bot to NyxID so your AI agents can receive and reply to messages.`}
              </p>
            </div>
            {children}
          </div>
          {email && (
            <div className="border-t border-border/60 px-5 py-3 text-center text-[11px] text-muted-foreground sm:px-8">
              Signed in as{" "}
              <span className="break-all text-foreground">{email}</span>
            </div>
          )}
        </main>
        <footer className="mt-6 flex h-7 items-center justify-center text-center text-[11px] text-muted-foreground">
          Channel connections by NyxID
        </footer>
      </div>
    </div>
  );
}
