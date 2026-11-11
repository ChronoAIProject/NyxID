import { getFormattingLocale } from "@/lib/formatting-locale";
import { useUiTranslation } from "@/i18n/use-ui-translation";
import { translateUi } from "@/i18n/use-ui-translation";
import { ChannelActivities, ActivityCallbackSettings } from "@/components/channels/channel-activities";
import { useChannelPlatformViews } from "@/hooks/use-channel-platforms";
import { zodResolver } from "@hookform/resolvers/zod";
import { toast } from "sonner";
import { useAppForm } from "@/components/ui/form";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Switch } from "@/components/ui/switch";
import { DetailSection } from "@/components/shared/detail-section";
import { useBreadcrumbLabel } from "@/components/layout/dashboard-layout";
import {
  useChannelConversation,
  useUpdateChannelConversation,
  useSendChannelMessage,
} from "@/hooks/use-channel-conversations";
import {
  channelInitiatedSettingsSchema,
  channelTestMessageSchema,
  type ChannelInitiatedSettingsFormData,
  type ChannelTestMessageFormData,
} from "@/schemas/channels";
import { useRef, useState } from "react";
import { useParams, useNavigate } from "@tanstack/react-router";
import { useChannelMessages } from "@/hooks/use-channel-messages";
import { useChannelBot } from "@/hooks/use-channel-bots";
import { cn } from "@/lib/utils";
import { ErrorBanner } from "@/components/shared/error-banner";
import { PageHeader } from "@/components/shared/page-header";
import { Skeleton } from "@/components/ui/skeleton";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  ChevronLeft,
  ChevronRight,
  ArrowDownLeft,
  ArrowUpRight,
} from "lucide-react";
import { MobileNotificationIcon } from "@/components/icons/empty-state";
import type {
  CallbackStatus,
  ChannelConversationItem,
  ChannelMessageItem,
  ChannelDelivery,
  ChannelDeliveryStatus,
  ContentType,
  MessageDirection,
} from "@/types/channels";

// -- Helpers --

function formatMessageTime(dateStr: string): string {
  const date = new Date(dateStr);
  if (Number.isNaN(date.getTime())) return "N/A";
  return new Intl.DateTimeFormat(getFormattingLocale(), {
    month: "short",
    day: "numeric",
    hour: "numeric",
    minute: "2-digit",
    hour12: true,
  }).format(date);
}

function deliveryBadgeVariant(
  status: CallbackStatus,
): "success" | "warning" | "destructive" | "secondary" {
  switch (status) {
    case "delivered":
      return "success";
    case "pending":
      return "warning";
    case "failed":
      return "destructive";
    case "timeout":
      return "secondary";
    default:
      return "secondary";
  }
}

function contentTypeLabel(ct: ContentType): string {
  switch (ct) {
    case "text":
      return translateUi("Text");
    case "image":
      return translateUi("Image");
    case "file":
      return translateUi("File");
    case "audio":
      return translateUi("Audio");
    case "video":
      return translateUi("Video");
    case "location":
      return translateUi("Location");
    case "sticker":
      return translateUi("Sticker");
    case "unknown":
      return translateUi("Unknown");
    default:
      return ct;
  }
}

function directionLabel(dir: MessageDirection): string {
  return dir === "inbound" ? translateUi("Inbound") : translateUi("Outbound");
}

const receiptLabels: Record<ChannelDeliveryStatus, string> = {
  accepted: "Platform accepted", sent: "Sent by WhatsApp", delivered: "Delivered",
  read: "Read", played: "Played", failed: "Delivery failed", partial: "Partially accepted",
  unknown: "Acceptance unknown", legacy_final_only: "Final part accepted",
  recipient_only: "Recipient observations only",
};

function DeliveryDetails({ delivery }: { readonly delivery: ChannelDelivery }) {
  useUiTranslation();
  const failed = delivery.status === "failed";
  const uncertain = delivery.status === "partial" || delivery.status === "unknown";
  return <div className="mt-2 min-w-0 space-y-2 text-13 text-muted-foreground">
    <Badge variant={failed ? "destructive" : uncertain ? "warning" : "secondary"} className="max-w-full whitespace-normal text-12">
      {receiptLabels[delivery.status]}
    </Badge>
    {delivery.status === "legacy_final_only" && <p>{translateUi("Only the final component ID was recorded. Delivery of the whole reply cannot be confirmed.")}</p>}
    {delivery.recipient_only && <p>{translateUi("These observations do not confirm delivery to every group participant.")}</p>}
    {uncertain && <p>{delivery.status === "partial" ? translateUi("Only part of the reply was confirmed as accepted.") : translateUi("WhatsApp acceptance could not be confirmed.")}{translateUi(" Check the conversation before sending again; some components may already have arrived.")}</p>}
    {failed && <p>{translateUi("WhatsApp reported a failure. Review the component status and provider codes below.")}</p>}
    {delivery.failure_code !== null && <p>{translateUi("Send error code (NyxID): ")}{delivery.failure_code}</p>}
    {delivery.expected_components !== null && <p>{delivery.components.length}{translateUi(" of ")}{delivery.expected_components}{translateUi(" component IDs recorded.")}{delivery.status === "accepted" && " Acceptance does not confirm recipient delivery."}</p>}
    {delivery.components.length > 0 && <details>
      <summary className="cursor-pointer">{translateUi("Component receipts")}</summary>
      <ol className="mt-2 space-y-3">
        {delivery.components.map((part, index) => <li key={part.platform_message_id} className="min-w-0 break-words">
          <p>{translateUi("Component ")}{index + 1}: {receiptLabels[part.status]}</p>
          <p className="break-all">{part.platform_message_id}</p>
          {([ ["Sent", part.sent_at], ["Delivered", part.delivered_at], ["Read", part.read_at], ["Played", part.played_at], ["Failed", part.failed_at] ] as const).map(([label, time]) => time && <p key={label}>{label}: <time dateTime={time}>{time}</time></p>)}
          {part.error_codes.length > 0 && <p>{translateUi("WhatsApp error codes: ")}{part.error_codes.join(", ")}</p>}
        </li>)}
      </ol>
    </details>}
  </div>;
}

// -- Message Card --

export function MessageCard({ message }: { readonly message: ChannelMessageItem }) {
  useUiTranslation();
  const isInbound = message.direction === "inbound";
  const senderName =
    message.sender_display_name ?? message.sender_platform_id ?? "Unknown";

  return (
    <div
      className={cn("flex w-full", isInbound ? "justify-start" : "justify-end")}
    >
      <div
        className={cn(
          "min-w-0 max-w-[90%] sm:max-w-[75%] rounded-xl px-4 py-3 shadow-sm",
          isInbound
            ? "bg-muted text-foreground"
            : "bg-overlay text-foreground",
        )}
      >
        {/* Header */}
        <div
          className={cn(
            "mb-1 flex items-center gap-2 text-13",
            isInbound ? "text-muted-foreground" : "text-muted-foreground",
          )}
        >
          {isInbound ? (
            <ArrowDownLeft className="h-3 w-3" />
          ) : (
            <ArrowUpRight className="h-3 w-3" />
          )}
          <span className="font-medium">
            {isInbound ? senderName : translateUi("Agent")}
          </span>
          <span>{directionLabel(message.direction)}</span>
        </div>

        {/* Content type only — message bodies are no longer stored per ADR-013 */}
        <div className="mb-1">
          <Badge variant="secondary" className="text-12">
            {contentTypeLabel(message.content_type)}
          </Badge>
        </div>
        <p className="text-14 italic text-muted-foreground">{translateUi("Content is not stored in NyxID. Ask the agent for the message body.")}</p>

        {/* Footer */}
        <div className="mt-2 flex flex-wrap items-center gap-2 text-12 text-muted-foreground">
          <span>{formatMessageTime(message.created_at)}</span>
          {isInbound && message.callback_status && (
            <Badge
              variant={deliveryBadgeVariant(message.callback_status)}
              className="text-12"
            >
              {message.callback_status === "delivered" ? translateUi("Callback accepted") : message.callback_status === "pending" ? translateUi("Callback pending") : message.callback_status === "timeout" ? translateUi("Callback timed out") : translateUi("Callback failed")}
            </Badge>
          )}
          {!isInbound && !message.delivery && message.platform_message_id && <Badge variant="secondary" className="text-12">{translateUi("Platform accepted")}</Badge>}
        </div>
        {!isInbound && message.delivery && <DeliveryDetails delivery={message.delivery} />}
        {!isInbound && !message.delivery && message.platform_message_id && <p className="mt-2 text-13 text-muted-foreground break-all">{translateUi("Platform message ID: ")}{message.platform_message_id}. Acceptance does not confirm recipient delivery.</p>}
      </div>
    </div>
  );
}

export function InitiatedMessageSettings({
  conversation,
  botOwnerId,
  botOwnerError = false,
  onRetryBotOwner,
}: {
  readonly conversation: ChannelConversationItem;
  /** Owner of the backing bot, whose credits a send spends; required to send. */
  readonly botOwnerId: string | undefined;
  /** The bot could not be loaded, so its owner is unknown until a retry. */
  readonly botOwnerError?: boolean;
  readonly onRetryBotOwner?: () => void;
}) {
  useUiTranslation();
  const update = useUpdateChannelConversation();
  const send = useSendChannelMessage({ creditsOwnerId: botOwnerId });
  const attempt = useRef<{ text: string; key: string } | null>(null);
  const settings = useAppForm<ChannelInitiatedSettingsFormData>({
    resolver: zodResolver(channelInitiatedSettingsSchema),
    values: {
      allow_agent_initiated: conversation.allow_agent_initiated ?? false,
    },
  });
  const message = useAppForm<ChannelTestMessageFormData>({
    resolver: zodResolver(channelTestMessageSchema),
    defaultValues: { text: "" },
    mode: "onChange",
  });
  const supported = conversation.capabilities?.initiated_send ?? false;
  const addressable =
    conversation.platform !== "device" &&
    Boolean(conversation.platform_conversation_id.trim()) &&
    conversation.platform_conversation_id !== "*";
  const unavailable = !supported
    ? "This platform does not support agent-initiated messages."
    : !addressable
      ? "This route has no specific chat address. Create a route for a specific chat to enable messages."
      : !conversation.is_active
        ? "Enable this conversation before sending messages."
        : null;

  return (
    <DetailSection title={translateUi("Agent-initiated messages")}>
      <div className="space-y-1 p-5 text-13 text-muted-foreground">
        <p>{translateUi("Editing: ")}{conversation.capabilities?.edit ? translateUi("Supported") : translateUi("Unavailable")}</p>
        <p>{translateUi("Receive media: ")}{conversation.capabilities?.media?.inbound.join(", ") || translateUi("None")}</p>
        <p>{translateUi("Send media: ")}{conversation.capabilities?.media?.outbound.join(", ") || translateUi("None")}</p>
      </div>
      <form
        className="space-y-4 p-5"
        onSubmit={settings.handleSubmit(async (values) => {
          try {
            await update.mutateAsync({ id: conversation.id, ...values });
            settings.reset(values);
            toast.success(translateUi("Message settings saved"));
          } catch {
            /* Mutation error is displayed below. */
          }
        })}
      >
        <div className="flex items-center justify-between gap-4 rounded-lg border border-border p-4">
          <div className="space-y-1">
            <Label htmlFor="allow-agent-initiated">{translateUi("Allow unprompted messages")}</Label>
            <p
              id="initiated-warning"
              className="text-14 text-muted-foreground"
            >{translateUi("This lets the assigned agent message this chat without anyone asking first, including alerts and scheduled updates.")}</p>
          </div>
          <Switch
            id="allow-agent-initiated"
            aria-describedby="initiated-warning"
            checked={settings.watch("allow_agent_initiated")}
            disabled={Boolean(unavailable) || update.isPending}
            onCheckedChange={(checked) =>
              settings.setValue("allow_agent_initiated", checked)
            }
          />
        </div>
        {unavailable && (
          <p className="text-14 text-muted-foreground">{unavailable}</p>
        )}
        {update.error && <ErrorBanner message={update.error.message} />}
        <div className="flex justify-end">
          <Button
            type="submit"
            variant="primary"
            isLoading={update.isPending}
            disabled={!settings.formState.isDirty || Boolean(unavailable)}
          >{translateUi("Save")}</Button>
        </div>
      </form>
      {supported && addressable && (
        <form
          className="space-y-3 p-5"
          onSubmit={(event) => {
            void message.handleSubmit(async ({ text }) => {
              // Enter submits even while the button is disabled.
              if (!botOwnerId) return;
              if (attempt.current?.text !== text)
                attempt.current = { text, key: crypto.randomUUID() };
              try {
                await send.mutateAsync({
                  conversation_id: conversation.id,
                  message: { text },
                  idempotency_key: attempt.current.key,
                });
                message.reset({ text: "" });
                attempt.current = null;
                toast.success(translateUi("Platform accepted the test message"));
              } catch {
                /* Keep the delivery key for an explicit retry of the same message. */
              }
            })(event);
          }}
        >
          <Label htmlFor="channel-test-message">{translateUi("Send test message")}</Label>
          <Input
            id="channel-test-message"
            {...message.register("text")}
            disabled={
              !conversation.allow_agent_initiated ||
              send.isPending ||
              !conversation.is_active ||
              !botOwnerId
            }
            placeholder={translateUi("Your test message")}
          />
          {!botOwnerId && botOwnerError ? (
            <ErrorBanner
              message={translateUi("This bot's details could not be loaded, so test messages can't be sent.")}
              onRetry={onRetryBotOwner}
            />
          ) : !conversation.allow_agent_initiated ? (
            <p className="text-14 text-muted-foreground">{translateUi("Save the setting above before sending a test message.")}</p>
          ) : (
            !botOwnerId && (
              <p className="text-14 text-muted-foreground">{translateUi("Sending is available once this bot's details load.")}</p>
            )
          )}
          {message.formState.errors.text && (
            <p role="alert" className="text-14 text-destructive">
              {message.formState.errors.text.message}
            </p>
          )}
          {send.error && <ErrorBanner message={send.error.message} />}
          <div className="flex justify-end">
            <Button
              type="submit"
              isLoading={send.isPending}
              disabled={
                !conversation.allow_agent_initiated ||
                !conversation.is_active ||
                !botOwnerId ||
                !message.formState.isValid
              }
            >{translateUi("Send test message")}</Button>
          </div>
        </form>
      )}
    </DetailSection>
  );
}

// -- Page --

export function ChannelConversationDetailPage() {
  useUiTranslation();
  const { botId, conversationId } = useParams({ strict: false }) as {
    botId: string;
    conversationId: string;
  };
  const navigate = useNavigate();

  const [page, setPage] = useState(1);
  const perPage = 50;

  const {
    data: bot,
    isError: botError,
    refetch: refetchBot,
  } = useChannelBot(botId);
  const { getPlatform } = useChannelPlatformViews();
  const activities = getPlatform(bot?.platform ?? "").activities;
  const { data: conversation, error: conversationError } =
    useChannelConversation(conversationId);
  useBreadcrumbLabel("Messages");
  useBreadcrumbLabel(bot?.label, `/channel-bots/${botId}`);
  const { data, isLoading, error, refetch } = useChannelMessages(
    conversationId,
    page,
    perPage,
  );

  const messages = data?.messages ?? [];
  const total = data?.total ?? 0;
  const totalPages = Math.max(1, Math.ceil(total / perPage));

  if (isLoading && activities.length === 0) {
    return (
      <div className="space-y-8">
        <Skeleton className="h-12 w-64" />
        <div className="space-y-3">
          {Array.from({ length: 6 }, (_, i) => (
            <Skeleton
              key={`msg-skel-${String(i)}`}
              className={cn(
                "h-16",
                i % 2 === 0 ? "mr-auto w-2/3" : "ml-auto w-2/3",
              )}
            />
          ))}
        </div>
      </div>
    );
  }

  return (
    <div className="space-y-8">
      <PageHeader
        title={activities.length ? translateUi("Channel activity") : translateUi("Messages")}
        description={activities.length ? translateUi("Conversation {{value1}}…", { value1: conversationId.slice(0, 12) }) : translateUi("Conversation {{value1}}... -- {{value2}} message{{value3}}", { value1: conversationId.slice(0, 12), value2: String(total), value3: total === 1 ? "" : "s" })}
        actions={
          <Button
            variant="outline"
            onClick={() =>
              void navigate({ to: `/channel-bots/${botId}` as string })
            }
          >{translateUi("Back to Bot")}</Button>
        }
      />

      {/* Conversation metadata */}
      {bot && (
        <div className="flex flex-wrap items-center gap-2 text-14 text-muted-foreground">
          <Badge variant="secondary">{bot.platform}</Badge>
          <span>{translateUi("Conversation ID:")}</span>
          <code className="rounded bg-muted px-1.5 py-0.5 font-mono text-14">
            {conversationId.slice(0, 16)}
          </code>
        </div>
      )}

      {conversationError && (
        <ErrorBanner message={translateUi("Failed to load conversation settings.")} />
      )}
      {activities.length > 0 && (
        <>
          <ChannelActivities scope="route" id={conversationId} descriptors={activities} />
          <ActivityCallbackSettings id={conversationId} descriptors={activities} />
        </>
      )}
      {conversation && (
        <InitiatedMessageSettings
          key={conversation.id}
          conversation={conversation}
          botOwnerId={bot?.user_id}
          botOwnerError={botError && !bot}
          onRetryBotOwner={() => void refetchBot()}
        />
      )}
      {activities.length > 0 && <h2 className="type-card-title">{translateUi("Message history")}</h2>}
      {/* Message list */}
      {error ? (
        <ErrorBanner
          message={translateUi("Failed to load messages. Please try again.")}
          onRetry={refetch}
        />
      ) : isLoading ? <p className="text-13 text-muted-foreground">{translateUi("Loading message history…")}</p> : messages.length === 0 && activities.length > 0 ? (
        <p className="text-13 text-muted-foreground">{translateUi("No ordinary messages in this route. Encrypted chat and own-post notifications appear in Received activities above.")}</p>
      ) : messages.length === 0 ? (
        <div className="flex flex-col items-center justify-center gap-1 py-12 text-center">
          <MobileNotificationIcon className="h-64 w-64 text-muted-foreground" />
          <div className="space-y-1">
            <p className="type-row-title text-muted-foreground">{translateUi("No Messages")}</p>
            <p className="text-13 text-muted-foreground">{translateUi("No messages in this conversation yet.")}</p>
          </div>
        </div>
      ) : (
        <>
          <div className="mx-auto w-full max-w-3xl space-y-3">
            {messages.map((msg) => (
              <MessageCard key={msg.id} message={msg} />
            ))}
          </div>

          {/* Pagination */}
          {totalPages > 1 && (
            <div className="flex items-center justify-between">
              <p className="text-13 text-text-tertiary">{translateUi("Showing ")}{String((page - 1) * perPage + 1)}-
                {String(Math.min(page * perPage, total))}{translateUi(" of ")}{String(total)}
              </p>
              <div className="flex items-center gap-2">
                <Button
                  variant="outline"
                  disabled={page <= 1}
                  onClick={() => setPage((p) => Math.max(1, p - 1))}
                >
                  <ChevronLeft className="h-4 w-4" />
                </Button>
                <span className="text-13 text-text-tertiary">{translateUi("Page ")}{String(page)}{translateUi(" of ")}{String(totalPages)}
                </span>
                <Button
                  variant="outline"
                  disabled={page >= totalPages}
                  onClick={() => setPage((p) => Math.min(totalPages, p + 1))}
                >
                  <ChevronRight className="h-4 w-4" />
                </Button>
              </div>
            </div>
          )}
        </>
      )}
    </div>
  );
}
