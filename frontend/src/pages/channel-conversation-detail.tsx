import { useState } from "react";
import { useParams, useNavigate } from "@tanstack/react-router";
import { useChannelMessages } from "@/hooks/use-channel-messages";
import { useChannelBot } from "@/hooks/use-channel-bots";
import { cn } from "@/lib/utils";
import { PageHeader } from "@/components/shared/page-header";
import { Skeleton } from "@/components/ui/skeleton";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  ChevronLeft,
  ChevronRight,
  MessageSquare,
  ArrowDownLeft,
  ArrowUpRight,
} from "lucide-react";
import type {
  CallbackStatus,
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

function MessageCard({
  message,
}: {
  readonly message: ChannelMessageItem;
}) {
  const isInbound = message.direction === "inbound";
  const senderName =
    message.sender_display_name ?? message.sender_platform_id ?? "Unknown";

  return (
    <div
      className={cn(
        "flex w-full",
        isInbound ? "justify-start" : "justify-end",
      )}
    >
      <div
        className={cn(
          "max-w-[75%] rounded-xl px-4 py-3 shadow-sm",
          isInbound
            ? "bg-muted text-foreground"
            : "bg-primary/10 text-foreground",
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
          <Badge variant="outline" className="text-[9px]">
            {contentTypeLabel(message.content_type)}
          </Badge>
        </div>
        <p className="text-sm italic text-muted-foreground">
          Content is not stored in NyxID. Ask the agent for the message body.
        </p>

        {/* Footer */}
        <div className="mt-2 flex items-center gap-2 text-[10px] text-muted-foreground">
          <span>{formatMessageTime(message.created_at)}</span>
          {!isInbound && message.callback_status && (
            <Badge
              variant={deliveryBadgeVariant(message.callback_status)}
              className="text-[9px]"
            >
              {message.callback_status}
            </Badge>
          )}
        </div>
      </div>
    </div>
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
  const { data, isLoading, error } = useChannelMessages(
    conversationId,
    page,
    perPage,
  );

  const messages = data?.messages ?? [];
  const total = data?.total ?? 0;
  const totalPages = Math.max(1, Math.ceil(total / perPage));

  const botLabel = bot?.label ?? "Bot";

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
        breadcrumbs={[
          { label: "Channel Bots", to: "/channel-bots" },
          { label: botLabel, to: `/channel-bots/${botId}` },
          { label: "Messages" },
        ]}
        title="Messages"
        description={`Conversation ${conversationId.slice(0, 12)}... -- ${String(total)} message${total === 1 ? "" : "s"}`}
        actions={
          <Button
            variant="outline"
            size="sm"
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
        <div className="flex flex-wrap items-center gap-2 text-sm text-muted-foreground">
          <Badge variant="outline">{bot.platform}</Badge>
          <span>Conversation ID:</span>
          <code className="rounded bg-muted px-1.5 py-0.5 font-mono text-xs">
            {conversationId.slice(0, 16)}
          </code>
        </div>
      )}

      {/* Message list */}
      {error ? (
        <div className="flex flex-col items-center justify-center py-12 text-center">
          <MessageSquare className="mb-4 h-12 w-12 text-muted-foreground/50" />
          <p className="text-sm text-muted-foreground">
            Failed to load messages. Please try again.
          </p>
        </div>
      ) : messages.length === 0 ? (
        <div className="flex flex-col items-center justify-center py-12 text-center">
          <MessageSquare className="mb-4 h-12 w-12 text-muted-foreground/50" />
          <p className="text-sm text-muted-foreground">
            No messages in this conversation yet.
          </p>
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
              <p className="text-sm text-muted-foreground">
                Showing {String((page - 1) * perPage + 1)}-
                {String(Math.min(page * perPage, total))} of {String(total)}
              </p>
              <div className="flex items-center gap-2">
                <Button
                  variant="outline"
                  size="sm"
                  disabled={page <= 1}
                  onClick={() => setPage((p) => Math.max(1, p - 1))}
                >
                  <ChevronLeft className="h-4 w-4" />
                </Button>
                <span className="text-sm">
                  Page {String(page)} of {String(totalPages)}
                </span>
                <Button
                  variant="outline"
                  size="sm"
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
