import { useContext, useRef, useState, useSyncExternalStore } from "react";
import { ArrowUpRight, MessageSquare } from "lucide-react";
import { Button } from "@/components/ui/button";
import { useUiTranslation } from "@/i18n/use-ui-translation";
import { assistantErrorSummary } from "@/lib/assistant/request-error";
import { isAdmittedSendError } from "@/lib/assistant/send-admission";
import type { NyxBotHandoff } from "@/schemas/nyxbot-handoff";
import { NyxBotHandoffContext } from "@/lib/assistant/nyxbot-handoff-context";
import {
  handoffReceipt,
  handoffReceiptKey,
  saveHandoffReceipt,
  subscribeHandoffReceipts,
} from "@/lib/assistant/nyxbot-handoff-receipts";

export function NyxBotHandoffCard({
  handoff,
  messageId,
}: {
  readonly handoff: NyxBotHandoff;
  readonly messageId?: string;
}) {
  const { t } = useUiTranslation();
  const controller = useContext(NyxBotHandoffContext);
  const busy = useRef(false);
  const [pending, setPending] = useState(false);
  const [conversationId, setConversationId] = useState<string>();
  const [error, setError] = useState<string>();
  const [accepted, setAccepted] = useState(false);
  const key =
    controller?.eligible &&
    controller.ownerUserId &&
    controller.sourceConversationId &&
    messageId
      ? handoffReceiptKey(
          controller.ownerUserId,
          controller.sourceConversationId,
          messageId,
          handoff,
        )
      : undefined;
  const receipt = useSyncExternalStore(
    subscribeHandoffReceipts,
    () => handoffReceipt(key),
    () => undefined,
  );
  const adoptedId =
    receipt?.state === "started" ? receipt.conversationId : conversationId;
  const uncertain =
    !adoptedId &&
    (accepted ||
      receipt?.state === "accepted" ||
      (receipt?.state === "pending" && !pending));

  async function continueTask() {
    if (!controller || !key || busy.current || uncertain) return;
    if (adoptedId) {
      controller.open(adoptedId);
      return;
    }
    busy.current = true;
    setPending(true);
    setError(undefined);
    let adopted = false;
    try {
      await controller.continue(
        handoff,
        (id) => {
          adopted = true;
          setConversationId(id);
          saveHandoffReceipt(key, { state: "started", conversationId: id });
        },
        key,
      );
    } catch (cause) {
      // Once a thread exists, open it to review progress rather than resending.
      if (!adopted && isAdmittedSendError(cause)) {
        const existing = handoffReceipt(key);
        if (existing?.state === "started") {
          setConversationId(existing.conversationId);
          return;
        }
        setAccepted(true);
        saveHandoffReceipt(key, { state: "accepted" });
        setError(
          t(
            "This task may already have started. Check your chats before trying again.",
          ),
        );
      } else if (!adopted) {
        setError(
          assistantErrorSummary(
            cause instanceof Error
              ? cause.message
              : t("Could not start NyxBot. Try again."),
          ),
        );
      }
    } finally {
      busy.current = false;
      setPending(false);
    }
  }

  return (
    <section
      aria-label={t("Continue with NyxBot")}
      className="my-3 space-y-3 rounded-xl border border-border bg-card p-4"
    >
      <div className="flex items-center gap-2 type-secondary-emphasis">
        <MessageSquare
          className="size-4 text-muted-foreground"
          aria-hidden="true"
        />
        {handoff.title}
      </div>
      <p className="whitespace-pre-wrap text-14 text-foreground/90">
        {handoff.task}
      </p>
      <p className="text-13 text-muted-foreground">
        {t(
          "Send this task as your message to NyxBot in a new chat. Review any required setup or approvals there.",
        )}
      </p>
      {error && (
        <p role="alert" className="text-13 text-destructive">
          {error}
        </p>
      )}
      {uncertain && !error && (
        <p role="status" className="text-13 text-muted-foreground">
          {t(
            "This task may already have started. Check your chats before trying again.",
          )}
        </p>
      )}
      <Button
        type="button"
        size="sm"
        disabled={!key || pending || uncertain}
        isLoading={pending}
        onClick={() => void continueTask()}
      >
        {adoptedId ? t("Open NyxBot chat") : t("Continue with NyxBot")}
        <ArrowUpRight className="size-4" aria-hidden="true" />
      </Button>
    </section>
  );
}
