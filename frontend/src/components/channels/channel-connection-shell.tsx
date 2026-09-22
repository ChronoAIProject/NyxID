import { useId, type ReactNode } from "react";
import { ExternalLink } from "lucide-react";
import { NyxidIcon } from "@/components/brand/nyxid-icon";
import { NyxidLogo } from "@/components/brand/nyxid-logo";
import { ServiceIcon } from "@/components/service-icon";
import { TelegramGlyph } from "@/components/service-icons/_shared";
import { useApplyTheme } from "@/hooks/use-theme";
import { cn } from "@/lib/utils";

const ARC_POINTS = Array.from({ length: 11 }, (_, index) => {
  const t = index / 10;
  return { x: 2 + 76 * t, y: 24 - 4 * 8 * t * (1 - t) };
});

function ChannelIcon({ platform }: { readonly platform: string }) {
  if (platform === "telegram" || platform === "telegram-new") {
    return <TelegramGlyph className="size-8 text-foreground" />;
  }
  if (platform === "whatsapp") {
    return <img src="/nyxbot-whatsapp.svg" alt="" className="size-8 dark:invert" />;
  }
  return (
    <ServiceIcon
      slug={`api-${platform === "x" ? "twitter" : platform}`}
      size="lg"
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
  const arcId = useId();
  const gradientId = `${arcId}-gradient`;
  const maskId = `${arcId}-mask`;
  return (
    <div className="flex min-h-dvh flex-col bg-background px-4 py-6 text-foreground sm:py-10">
      <div className="m-auto w-full max-w-[520px]">
        <header className="mb-6 flex items-center justify-between gap-4">
          <NyxidLogo className="h-7 w-auto" />
          {guideUrl && (
            <a
              href={guideUrl}
              target="_blank"
              rel="noopener noreferrer"
              className="inline-flex items-center gap-1.5 text-[11px] text-muted-foreground hover:text-foreground"
            >
              Channel setup guide
              <ExternalLink className="size-3" aria-hidden="true" />
            </a>
          )}
        </header>
        <main className="overflow-hidden rounded-xl border border-border bg-card">
          <div className="px-5 py-8 sm:px-8">
            <div
              className="mb-6 flex items-start justify-center"
              role="img"
              aria-label={`NyxID connects to ${platformName ?? "your channel"}`}
            >
              <div className="flex w-20 flex-col items-center gap-2.5">
                <div className="flex size-16 items-center justify-center rounded-xl border border-border bg-background">
                  <NyxidIcon className="size-8" alt="" />
                </div>
                <span className="text-[12px] font-medium">NyxID</span>
              </div>
              <svg
                viewBox="0 0 80 32"
                className={cn(
                  "mt-2 h-8 w-20 shrink-0",
                  complete ? "text-success/60" : "text-muted-foreground/50",
                )}
                fill="none"
                aria-hidden="true"
              >
                <defs>
                  <linearGradient id={gradientId}>
                    <stop offset="0" stopColor="white" stopOpacity="0" />
                    <stop offset="0.35" stopColor="white" stopOpacity="0.25" />
                    <stop offset="0.7" stopColor="white" stopOpacity="0.9" />
                    <stop offset="1" stopColor="white" stopOpacity="0" />
                  </linearGradient>
                  <mask
                    id={maskId}
                    maskUnits="userSpaceOnUse"
                    x="0"
                    y="0"
                    width="80"
                    height="32"
                  >
                    <rect
                      width="40"
                      height="32"
                      fill={`url(#${gradientId})`}
                      className="channel-connection-sweep"
                    />
                  </mask>
                </defs>
                <g fill="currentColor">
                  {ARC_POINTS.map(({ x, y }, index) => (
                    <circle key={index} cx={x} cy={y} r="1.1" />
                  ))}
                </g>
                {!complete && (
                  <g
                    fill="currentColor"
                    className="text-foreground/80"
                    mask={`url(#${maskId})`}
                  >
                    {ARC_POINTS.map(({ x, y }, index) => (
                      <circle key={index} cx={x} cy={y} r="1.1" />
                    ))}
                  </g>
                )}
              </svg>
              <div className="flex w-20 flex-col items-center gap-2.5">
                <div className="flex size-16 items-center justify-center rounded-xl border border-border bg-background">
                  <ChannelIcon platform={platform} />
                </div>
                <span className="text-center text-[12px] font-medium">
                  {platformName ?? "Channel"}
                </span>
              </div>
            </div>
            <div className="mb-7 space-y-2 text-center">
              <h1 className="font-display text-[22px] font-medium leading-tight tracking-tight sm:text-[28px]">
                {platformName
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
