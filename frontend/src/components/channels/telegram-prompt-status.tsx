import { useUiTranslation } from "@/i18n/use-ui-translation";
import { translateUi } from "@/i18n/use-ui-translation";
import { ExternalLink } from "lucide-react";
import { Button } from "@/components/ui/button";
import type { TelegramNewRequest } from "@/schemas/telegram-new";

export function TelegramPromptStatus({ request, pending, recovering, onRecover, onRegenerate, regenerating, recoveryUrl, recoveryUnexpired }: {
  readonly request: TelegramNewRequest;
  readonly recoveryUnexpired: boolean;
  readonly pending: boolean;
  readonly recovering: boolean;
  readonly onRecover: (index?: number) => void;
  readonly onRegenerate: () => void;
  readonly regenerating: boolean;
  readonly recoveryUrl?: string;
}) {
  useUiTranslation();
  const delivery = request.prompt_delivery;
  const outcome = delivery?.outcome;
  const text = request.status === "waiting_telegram"
    ? translateUi("Open Telegram and press Start to link this setup to your Telegram account.")
    : !delivery
      ? translateUi("Delivery details were not recorded for this setup. Continue in Telegram to create your bot.")
      : outcome === "sending"
      ? translateUi("Sending the creation instructions and button to Telegram…")
      : outcome === "rejected"
        ? translateUi("Telegram rejected the creation message. NyxID has not yet confirmed bot creation.")
        : outcome === "accepted" && delivery?.phase === "keyboard"
          ? translateUi("Telegram accepted the Create bot message. This does not confirm the button appeared in your app.")
          : translateUi("Delivery of the Create bot message is unconfirmed. NyxID has not yet confirmed bot creation.");
  const reason = delivery?.reason === "rate_limited"
    ? translateUi("Telegram is limiting requests. Wait before sending a new /start.")
    : delivery?.reason === "unauthorized" || delivery?.reason === "forbidden"
      ? translateUi("An administrator may need to check the manager configuration.")
      : undefined;
  return (
    <div className="space-y-3 text-13 text-muted-foreground">
      <p role="status" aria-live="polite">{text}{reason ? ` ${reason}` : ""}</p>
      {request.can_open_creation_form && !recoveryUnexpired && <p role="status">{translateUi("This setup expired. Refresh its progress before continuing.")}</p>}
      {request.status === "waiting_bot" && <p>{translateUi("Waiting for a verified bot creation from the Telegram account that opened this setup.")}</p>}
      {request.can_open_creation_form && recoveryUnexpired && request.status === "waiting_bot" && (
        <>
          <p>{translateUi("Already submitted Telegram’s form? Check progress before trying again. If creation is still unconfirmed, wait and check again; do not submit another form.")}</p>
          <p>{translateUi("Button missing? Open the same creation form for ")}<strong className="break-words">{request.label}</strong>{translateUi(". Use the same Telegram account and check the NyxID destination shown in the setup chat.")}</p>
          <p>{translateUi("If Create bot or the native form will not open, use the same Telegram account on an up-to-date official Telegram client that supports managed creation, such as the official mobile app, and open this manager’s existing private chat. Show its bot keyboard and tap Create bot; if the keyboard is missing, send /start once to retry this same live setup and check the destination shown above.")}</p>
          <p>{translateUi("Keep this setup and destination. Do not restart NyxBot, create another assistant or start another setup request.")}</p>
          {request.username_suggestions && (
            <div className="space-y-2 type-secondary" aria-label={translateUi("Username suggestions")}>
              <p>{translateUi("Suggested username: ")}<code>{request.username_suggestions[0]}</code></p>
              <p>{translateUi("Use 5–32 letters, digits or underscores, start with a letter, and end in bot. Usernames must be unique. Telegram checks availability when you create the bot; these suggestions have not been checked.")}</p>
              <div className="flex flex-wrap gap-2">
                {request.username_suggestions.slice(1).map((username, index) => (
                  <Button key={username} type="button" variant="outline" disabled={pending} onClick={() => onRecover(index + 1)}>{translateUi("Use ")}{username}
                  </Button>
                ))}
                <Button type="button" variant="outline" disabled={pending} isLoading={regenerating} onClick={onRegenerate}>{translateUi("Regenerate username")}</Button>
              </div>
            </div>
          )}
          <Button type="button" variant="outline" disabled={pending} isLoading={recovering} onClick={() => onRecover(0)}>{translateUi("Open creation form ")}<ExternalLink className="size-3" aria-hidden="true" />
          </Button>
          {recoveryUrl && <Button asChild variant="outline"><a className="ph-no-capture" href={recoveryUrl} target="_blank" rel="noopener noreferrer" referrerPolicy="no-referrer">{translateUi("Open Telegram creation form")}</a></Button>}
        </>
      )}
    </div>
  );
}
