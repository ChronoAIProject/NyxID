import { ExternalLink } from "lucide-react";
import { useUiTranslation } from "@/i18n/use-ui-translation";
import { Button } from "@/components/ui/button";
import type { ChannelPlatform } from "@/types/channels";

export function LarkEventSetupLink({
  platform,
  permissionSetupUrl,
}: {
  readonly platform: ChannelPlatform;
  readonly permissionSetupUrl?: string | null;
}) {
  const { t } = useUiTranslation();
  if ((platform !== "lark" && platform !== "feishu") || !permissionSetupUrl)
    return null;

  let eventSetupUrl: string;
  try {
    // Both console pages belong to the app selected by the server.
    eventSetupUrl = new URL("event", permissionSetupUrl).href;
  } catch {
    return null;
  }

  return (
    <div className="space-y-2">
      <p className="text-13 text-muted-foreground">
        {t("Paste the callback URL, then subscribe to {{messageEvent}} and {{cardEvent}}.", {
          messageEvent: "im.message.receive_v1",
          cardEvent: "card.action.trigger",
        })}
      </p>
      <Button variant="outline" asChild>
        <a href={eventSetupUrl} target="_blank" rel="noopener noreferrer">
          {t("Open Events & Callbacks")}
          <ExternalLink className="size-4" aria-hidden="true" />
        </a>
      </Button>
    </div>
  );
}
