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
  ContentType,
  MessageDirection,
} from "@/types/channels";

// -- Helpers --

function formatMessageTime(dateStr: string): string {
  const date = new Date(dateStr);
  if (Number.isNaN(date.getTime())) return "N/A";
  return new Intl.DateTimeFormat("en-US", {
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
      return "Text";
    case "image":
      return "Image";
    case "file":
      return "File";
    case "audio":
      return "Audio";
    case "video":
      return "Video";
    case "location":
      return "Location";
    case "sticker":
      return "Sticker";
    case "unknown":
      return "Unknown";
    default:
      return ct;
  }
}

function directionLabel(dir: MessageDirection): string {
  return dir === "inbound" ? "Inbound" : "Outbound";
}

// -- Message Card --

function MessageCard({ message }: { readonly message: ChannelMessageItem }) {
  const isInbound = message.direction === "inbound";
  const senderName =
    message.sender_display_name ?? message.sender_platform_id ?? "Unknown";

  return (
    <div
      className={cn("flex w-full", isInbound ? "justify-start" : "justify-end")}
    >
      <div
        className={cn(
          "max-w-[75%] rounded-xl px-4 py-3 shadow-sm",
          isInbound
            ? "bg-muted text-foreground"
            : "bg-white/[0.03] text-foreground",
        )}
      >
        {/* Header */}
        <div
          className={cn(
            "mb-1 flex items-center gap-2 text-xs",
            isInbound ? "text-muted-foreground" : "text-muted-foreground",
          )}
        >
          {isInbound ? (
            <ArrowDownLeft className="h-3 w-3" />
          ) : (
            <ArrowUpRight className="h-3 w-3" />
          )}
          <span className="font-medium">
            {isInbound ? senderName : "Agent"}
          </span>
          <span>{directionLabel(message.direction)}</span>
        </div>

        {/* Content type only — message bodies are no longer stored per ADR-013 */}
        <div className="mb-1">
          <Badge variant="secondary" className="text-[10px]">
            {contentTypeLabel(message.content_type)}
          </Badge>
        </div>
        <p className="text-[12px] italic text-muted-foreground">
          Content is not stored in NyxID. Ask the agent for the message body.
        </p>

        {/* Footer */}
        <div className="mt-2 flex items-center gap-2 text-[10px] text-muted-foreground">
          <span>{formatMessageTime(message.created_at)}</span>
          {!isInbound && message.callback_status && (
            <Badge
              variant={deliveryBadgeVariant(message.callback_status)}
              className="text-[10px]"
            >
              {message.callback_status.charAt(0).toUpperCase() +
                message.callback_status.slice(1)}
            </Badge>
          )}
        </div>
      </div>
    </div>
  );
}

export function InitiatedMessageSettings({
  conversation,
}: {
  readonly conversation: ChannelConversationItem;
}) {
  const update = useUpdateChannelConversation();
  const send = useSendChannelMessage();
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
    <DetailSection title="Agent-initiated messages">
      <form
        className="space-y-4 p-5"
        onSubmit={settings.handleSubmit(async (values) => {
          try {
            await update.mutateAsync({ id: conversation.id, ...values });
            settings.reset(values);
            toast.success("Message settings saved");
          } catch {
            /* Mutation error is displayed below. */
          }
        })}
      >
        <div className="flex items-center justify-between gap-4 rounded-lg border border-border p-4">
          <div className="space-y-1">
            <Label htmlFor="allow-agent-initiated">
              Allow unprompted messages
            </Label>
            <p
              id="initiated-warning"
              className="text-[12px] text-muted-foreground"
            >
              This lets the assigned agent message this chat without anyone
              asking first, including alerts and scheduled updates.
            </p>
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
          <p className="text-[12px] text-muted-foreground">{unavailable}</p>
        )}
        {update.error && <ErrorBanner message={update.error.message} />}
        <div className="flex justify-end">
          <Button
            type="submit"
            variant="primary"
            isLoading={update.isPending}
            disabled={!settings.formState.isDirty || Boolean(unavailable)}
          >
            Save
          </Button>
        </div>
      </form>
      {supported && addressable && (
        <form
          className="space-y-3 p-5"
          onSubmit={(event) => {
            void message.handleSubmit(async ({ text }) => {
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
                toast.success("Platform accepted the test message");
              } catch {
                /* Keep the delivery key for an explicit retry of the same message. */
              }
            })(event);
          }}
        >
          <Label htmlFor="channel-test-message">Send test message</Label>
          <Input
            id="channel-test-message"
            {...message.register("text")}
            disabled={
              !conversation.allow_agent_initiated ||
              send.isPending ||
              !conversation.is_active
            }
            placeholder="Your test message"
          />
          {!conversation.allow_agent_initiated && (
            <p className="text-[12px] text-muted-foreground">
              Save the setting above before sending a test message.
            </p>
          )}
          {message.formState.errors.text && (
            <p role="alert" className="text-[12px] text-destructive">
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
                !message.formState.isValid
              }
            >
              Send test message
            </Button>
          </div>
        </form>
      )}
    </DetailSection>
  );
}

// -- Page --

export function ChannelConversationDetailPage() {
  const { botId, conversationId } = useParams({ strict: false }) as {
    botId: string;
    conversationId: string;
  };
  const navigate = useNavigate();

  const [page, setPage] = useState(1);
  const perPage = 50;

  const { data: bot } = useChannelBot(botId);
  const { data: conversation, error: conversationError } =
    useChannelConversation(conversationId);
  useBreadcrumbLabel(bot ? `${bot.label} messages` : "Messages");
  const { data, isLoading, error, refetch } = useChannelMessages(
    conversationId,
    page,
    perPage,
  );

  const messages = data?.messages ?? [];
  const total = data?.total ?? 0;
  const totalPages = Math.max(1, Math.ceil(total / perPage));

  if (isLoading) {
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
        title="Messages"
        description={`Conversation ${conversationId.slice(0, 12)}... -- ${String(total)} message${total === 1 ? "" : "s"}`}
        actions={
          <Button
            variant="outline"
            onClick={() =>
              void navigate({ to: `/channel-bots/${botId}` as string })
            }
          >
            Back to Bot
          </Button>
        }
      />

      {/* Conversation metadata */}
      {bot && (
        <div className="flex flex-wrap items-center gap-2 text-[12px] text-muted-foreground">
          <Badge variant="secondary">{bot.platform}</Badge>
          <span>Conversation ID:</span>
          <code className="rounded bg-muted px-1.5 py-0.5 font-mono text-xs">
            {conversationId.slice(0, 16)}
          </code>
        </div>
      )}

      {conversationError && (
        <ErrorBanner message="Failed to load conversation settings." />
      )}
      {conversation && (
        <InitiatedMessageSettings
          key={conversation.id}
          conversation={conversation}
        />
      )}

      {/* Message list */}
      {error ? (
        <ErrorBanner
          message="Failed to load messages. Please try again."
          onRetry={refetch}
        />
      ) : messages.length === 0 ? (
        <div className="flex flex-col items-center justify-center gap-1 py-12 text-center">
          <MobileNotificationIcon className="h-64 w-64 text-muted-foreground" />
          <div className="space-y-1">
            <p className="text-[12px] font-medium text-muted-foreground">
              No Messages
            </p>
            <p className="text-xs text-muted-foreground">
              No messages in this conversation yet.
            </p>
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
              <p className="text-[11px] text-text-tertiary">
                Showing {String((page - 1) * perPage + 1)}-
                {String(Math.min(page * perPage, total))} of {String(total)}
              </p>
              <div className="flex items-center gap-2">
                <Button
                  variant="outline"
                  disabled={page <= 1}
                  onClick={() => setPage((p) => Math.max(1, p - 1))}
                >
                  <ChevronLeft className="h-4 w-4" />
                </Button>
                <span className="text-[11px] text-text-tertiary">
                  Page {String(page)} of {String(totalPages)}
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
