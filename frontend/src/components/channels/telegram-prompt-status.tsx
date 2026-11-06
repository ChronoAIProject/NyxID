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
  const delivery = request.prompt_delivery;
  const outcome = delivery?.outcome;
  const text = request.status === "waiting_telegram"
    ? "Open Telegram and press Start to link this setup to your Telegram account."
    : !delivery
      ? "Delivery details were not recorded for this setup. Continue in Telegram to create your bot."
      : outcome === "sending"
      ? "Sending the creation instructions and button to Telegram…"
      : outcome === "rejected"
        ? "Telegram rejected the creation message. NyxID has not yet confirmed bot creation."
        : outcome === "accepted" && delivery?.phase === "keyboard"
          ? "Telegram accepted the Create bot message. This does not confirm the button appeared in your app."
          : "Delivery of the Create bot message is unconfirmed. NyxID has not yet confirmed bot creation.";
  const reason = delivery?.reason === "rate_limited"
    ? "Telegram is limiting requests. Wait before sending a new /start."
    : delivery?.reason === "unauthorized" || delivery?.reason === "forbidden"
      ? "An administrator may need to check the manager configuration."
      : undefined;
  return (
    <div className="space-y-3 text-13 text-muted-foreground">
      <p role="status" aria-live="polite">{text}{reason ? ` ${reason}` : ""}</p>
      {request.can_open_creation_form && !recoveryUnexpired && <p role="status">This setup expired. Refresh its progress before continuing.</p>}
      {request.status === "waiting_bot" && <p>Waiting for a verified bot creation from the Telegram account that opened this setup.</p>}
      {request.can_open_creation_form && recoveryUnexpired && request.status === "waiting_bot" && (
        <>
          <p>Button missing? Open the same creation form for <strong className="break-words">{request.label}</strong>. Use the same Telegram account and check the NyxID destination shown in the setup chat.</p>
          {request.username_suggestions && (
            <div className="space-y-2 type-secondary" aria-label="Username suggestions">
              <p>Suggested username: <code>{request.username_suggestions[0]}</code></p>
              <p>Use 5–32 letters, digits or underscores, start with a letter, and end in bot. Usernames must be unique. Telegram checks availability when you create the bot; these suggestions have not been checked.</p>
              <div className="flex flex-wrap gap-2">
                {request.username_suggestions.slice(1).map((username, index) => (
                  <Button key={username} type="button" variant="outline" disabled={pending} onClick={() => onRecover(index + 1)}>
                    Use {username}
                  </Button>
                ))}
                <Button type="button" variant="outline" disabled={pending} isLoading={regenerating} onClick={onRegenerate}>
                  Regenerate username
                </Button>
              </div>
            </div>
          )}
          <Button type="button" variant="outline" disabled={pending} isLoading={recovering} onClick={() => onRecover(0)}>
            Open creation form <ExternalLink className="size-3" aria-hidden="true" />
          </Button>
          {recoveryUrl && <Button asChild variant="outline"><a className="ph-no-capture" href={recoveryUrl} target="_blank" rel="noopener noreferrer" referrerPolicy="no-referrer">Open Telegram creation form</a></Button>}
        </>
      )}
    </div>
  );
}
