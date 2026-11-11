import { useUiTranslation } from "@/i18n/use-ui-translation";
import { translateUi } from "@/i18n/use-ui-translation";
import { translateErrorMessage } from "@/lib/error-messages";
import { LarkBotSetupStatus, LARK_VERIFY_SUCCESS } from "@/components/channels/lark-bot-setup-status";
import { ChannelActivities, LatestActivity } from "@/components/channels/channel-activities";
import { WhatsappNativeSection } from "@/components/channels/whatsapp-native";
import { useChannelActivities } from "@/hooks/use-channel-activities";
import type { ChannelActivityDescriptor, ChannelActivityItem } from "@/types/channels";
import { useChannelPlatformViews } from "@/hooks/use-channel-platforms";
import { useEffect, useState } from "react";
import { useParams, useNavigate, Link } from "@tanstack/react-router";
import { useAppForm } from "@/components/ui/form";
import { zodResolver } from "@hookform/resolvers/zod";
import {
  useChannelBot,
  useDeleteChannelBot,
  useUpdateChannelBot,
  useVerifyChannelBot,
} from "@/hooks/use-channel-bots";
import {
  useChannelConversations,
  useCreateChannelConversation,
  useDeleteChannelConversation,
} from "@/hooks/use-channel-conversations";
import { useApiKeys } from "@/hooks/use-api-keys";
import { useOrgs } from "@/hooks/use-orgs";
import { useAuthStore } from "@/stores/auth-store";
import {
  createChannelConversationSchema,
  updateChannelBotSchema,
  type CreateChannelConversationFormData,
  type UpdateChannelBotFormData,
} from "@/schemas/channels";
import { ApiError } from "@/lib/api-client";
import { editableChannelFields, TELEGRAM_MANAGER_DELETION_NOTE } from "@/lib/channel-platforms";
import { cn, formatDate, formatRelativeTime } from "@/lib/utils";
import { useRuntimeConfig } from "@/hooks/use-runtime-config";
import { MANAGED_FLOW_COMPONENTS } from "@/components/channels/managed-flows";
import { CredentialVerification, RouteReadiness } from "@/components/channels/bot-readiness";
import { routeAgentIssue, verificationErrorMessage } from "@/lib/channel-readiness";
import { PageHeader } from "@/components/shared/page-header";
import { CopyableUrlCallout } from "@/components/shared/copyable-url-callout";
import { useBreadcrumbLabel } from "@/components/layout/dashboard-layout";
import { ErrorBanner } from "@/components/shared/error-banner";
import { DetailSection } from "@/components/shared/detail-section";
import { OwnershipTransferCard } from "@/components/shared/ownership-transfer-card";
import { DetailRow } from "@/components/shared/detail-row";
import { Skeleton } from "@/components/ui/skeleton";
import { Badge } from "@/components/ui/badge";
import { Button, ButtonIcon } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Switch } from "@/components/ui/switch";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import {
  Check,
  Circle,
  ExternalLink,
  Loader2,
  MessageSquare,
  MoreVertical,
  Pencil,
  ShieldCheck,
  Trash2,
} from "lucide-react";
import { ArticleIcon } from "@/components/icons/empty-state";
import { AddCtaButton } from "@/components/shared/add-cta-button";
import { toast } from "sonner";
import type {
  ChannelBotDetail,
  ChannelConversationItem,
} from "@/types/channels";
import {
  conversationTypeLabel,
  statusBadgeVariant,
  statusLabel,
} from "./channel-bot-detail.helpers";

function ConversationRow({
  conversation,
  apiKeyNames,
  botId,
  onDelete,
  activities,
  latest,
  activityError,
  activityLoading,
}: {
  readonly conversation: ChannelConversationItem;
  readonly apiKeyNames: ReadonlyMap<string, string>;
  readonly botId: string;
  readonly onDelete: (id: string) => void;
  readonly activities: readonly ChannelActivityDescriptor[];
  readonly latest?: ChannelActivityItem;
  readonly activityError: boolean;
  readonly activityLoading: boolean;
}) {
  useUiTranslation();
  const agentName =
    apiKeyNames.get(conversation.agent_api_key_id) ??
    conversation.agent_api_key_id.slice(0, 8);

  return (
    <TableRow>
      <TableCell className="text-14">
        {conversation.platform_conversation_id ||
          conversation.platform_sender_id ||
          "-"}
      </TableCell>
      <TableCell>
        <Badge variant="secondary">
          {conversationTypeLabel(conversation.platform_conversation_type)}
        </Badge>
      </TableCell>
      <TableCell className="font-normal">{agentName}</TableCell>
      <TableCell>
        {conversation.default_agent ? (
          <Badge variant="info">{translateUi("Default")}</Badge>
        ) : (
          <span className="text-13 text-muted-foreground">-</span>
        )}
      </TableCell>
      <TableCell>
        {conversation.is_active ? (
          <Badge variant="success">{translateUi("Active")}</Badge>
        ) : (
          <Badge variant="secondary">{translateUi("Inactive")}</Badge>
        )}
      </TableCell>
      <TableCell className="text-14 text-muted-foreground">
        {activities.length ? <LatestActivity activity={latest} descriptors={activities} unavailable={activityError} loading={activityLoading} /> : conversation.last_message_at
          ? formatRelativeTime(conversation.last_message_at)
          : translateUi("Never")}
      </TableCell>
      <TableCell className="w-10">
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <Button variant="ghost" size="icon" className="h-7 w-7">
              <MoreVertical className="h-3.5 w-3.5" aria-hidden="true" />
              <span className="sr-only">{translateUi("Actions")}</span>
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end">
            <DropdownMenuItem asChild>
              <Link
                to={`/channel-bots/${botId}/conversations/${conversation.id}` as string}
              >
                <MessageSquare className="mr-2 h-4 w-4" aria-hidden="true" />{translateUi("View messages")}</Link>
            </DropdownMenuItem>
            <DropdownMenuItem
              onClick={() => onDelete(conversation.id)}
              className="text-destructive focus:text-destructive"
            >
              <Trash2 className="mr-2 h-4 w-4 text-destructive" aria-hidden="true" />{translateUi("Delete")}</DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>
      </TableCell>
    </TableRow>
  );
}

function ConversationsSection({
  botId,
  platform,
  apiKeyNames,
  ownerOrgId,
  ownerLabel,
  isTelegramManager,
}: {
  readonly botId: string;
  readonly platform: string;
  readonly apiKeyNames: ReadonlyMap<string, string>;
  /** When the parent bot is org-owned, the org id (a user_id). Used to
   *  scope the conversation list and pre-fill `target_org_id` on create.
   *  `null` means personal. */
  readonly ownerOrgId: string | null;
  readonly ownerLabel: string;
  readonly isTelegramManager: boolean;
}) {
  useUiTranslation();
  const { data: conversations, isLoading, error } = useChannelConversations({
    botId,
    orgId: ownerOrgId,
  });
  const { getPlatform } = useChannelPlatformViews();
  const activities = getPlatform(platform).activities;
  const activityQuery = useChannelActivities("bot", botId, activities.length > 0);
  const latest = (id: string) => activityQuery.data?.routes.find((entry) => entry.conversation_id === id)?.last_activity;
  const keys = useApiKeys({ orgId: ownerOrgId });
  const [addOpen, setAddOpen] = useState(false);
  const [deleteTarget, setDeleteTarget] = useState<string | null>(null);

  return (
    <div className="space-y-4">
      {activities.length > 0 && <ChannelActivities scope="bot" id={botId} descriptors={activities} />}
      <div className="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
        <div className="min-w-0">
          <h3 className="type-card-title">{translateUi("Conversation Routes")}</h3>
          <p className="text-14 text-muted-foreground">{translateUi("Map conversations to AI agents for message relay.")}</p>
        </div>
        <div className="shrink-0">
          <AddCtaButton label={translateUi("Add Route")} onClick={() => setAddOpen(true)} />
        </div>
      </div>

      {keys.error ? <ErrorBanner message={translateUi("Agent keys could not be loaded for this owner. Retry the page.")} /> : !keys.isLoading && keys.data && <RouteReadiness keys={keys.data} ownerOrgId={ownerOrgId} ownerLabel={ownerLabel} conversations={conversations} />}

      {error ? <ErrorBanner message={translateUi("Conversation routes could not be loaded. Retry the page.")} /> : isLoading ? (
        <div className="space-y-2">
          {Array.from({ length: 2 }, (_, i) => (
            <Skeleton key={`conv-skel-${String(i)}`} className="h-12 w-full" />
          ))}
        </div>
      ) : !conversations || conversations.length === 0 ? (
        <div className="flex flex-col items-center justify-center gap-1 py-8 text-center">
          <ArticleIcon className="h-48 w-48 text-muted-foreground" />
          <p className="text-14 text-muted-foreground">{translateUi("No conversation routes configured. Add a route to start relaying messages to an AI agent.")}</p>
        </div>
      ) : (
        <>
          {/* Mobile card view */}
          <div className="flex flex-col gap-3 md:hidden">
            {conversations.map((conv) => {
              const agentName = apiKeyNames.get(conv.agent_api_key_id) ?? conv.agent_api_key_id.slice(0, 8);
              return (
                <div key={conv.id} className="relative rounded-xl border border-border/50 bg-card p-4">
                  <div className="absolute right-3 top-3" onClick={(e) => e.stopPropagation()} onKeyDown={(e) => e.stopPropagation()}>
                    <DropdownMenu>
                      <DropdownMenuTrigger asChild>
                        <Button variant="ghost" size="icon" className="h-7 w-7">
                          <MoreVertical className="h-3.5 w-3.5" />
                        </Button>
                      </DropdownMenuTrigger>
                      <DropdownMenuContent align="end">
                        <DropdownMenuItem asChild>
                          <Link to={`/channel-bots/${botId}/conversations/${conv.id}` as string}>
                            <MessageSquare className="mr-2 h-4 w-4" />{translateUi(" View messages")}</Link>
                        </DropdownMenuItem>
                        <DropdownMenuItem onClick={() => setDeleteTarget(conv.id)} className="text-destructive focus:text-destructive">
                          <Trash2 className="mr-2 h-4 w-4 text-destructive" />{translateUi(" Delete")}</DropdownMenuItem>
                      </DropdownMenuContent>
                    </DropdownMenu>
                  </div>
                  <p className="pr-10 type-card-title text-foreground truncate">
                    {conv.platform_conversation_id || conv.platform_sender_id || "—"}
                  </p>
                  <p className="text-13 text-muted-foreground">{translateUi("Agent: ")}{agentName}</p>
                  <div className="mt-2 flex flex-wrap gap-1.5">
                    <Badge variant="secondary">{conversationTypeLabel(conv.platform_conversation_type)}</Badge>
                    {conv.is_active ? <Badge variant="success">{translateUi("Active")}</Badge> : <Badge variant="secondary">{translateUi("Inactive")}</Badge>}
                    {conv.default_agent && <Badge variant="info">{translateUi("Default")}</Badge>}
                  </div>
                  <div className="mt-3 text-13 text-muted-foreground">
                    {activities.length ? <LatestActivity activity={latest(conv.id)} descriptors={activities} unavailable={!!activityQuery.error} loading={activityQuery.isPending} /> : conv.last_message_at ? translateUi("Last message {{value1}}", { value1: formatRelativeTime(conv.last_message_at) }) : translateUi("No messages")}
                  </div>
                </div>
              );
            })}
          </div>

          {/* Desktop table view */}
          <div className="hidden md:block rounded-xl border border-border/50 bg-card overflow-hidden">
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>{translateUi("Conversation ID")}</TableHead>
                  <TableHead>{translateUi("Type")}</TableHead>
                  <TableHead>{translateUi("Agent")}</TableHead>
                  <TableHead>{translateUi("Default")}</TableHead>
                  <TableHead>{translateUi("Status")}</TableHead>
                  <TableHead>{activities.length ? translateUi("Latest activity") : translateUi("Last Message")}</TableHead>
                  <TableHead className="w-10">{translateUi("Actions")}</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {conversations.map((conv) => (
                  <ConversationRow
                    key={conv.id}
                    conversation={conv}
                    apiKeyNames={apiKeyNames}
                    botId={botId}
                    onDelete={setDeleteTarget}
                    activities={activities}
                    latest={latest(conv.id)}
                    activityError={!!activityQuery.error}
                    activityLoading={activityQuery.isPending}
                  />
                ))}
              </TableBody>
            </Table>
          </div>
        </>
      )}

      <AddRouteDialog
        open={addOpen}
        onOpenChange={setAddOpen}
        botId={botId}
        ownerOrgId={ownerOrgId}
        ownerLabel={ownerLabel}
        isTelegramManager={isTelegramManager}
      />
      <DeleteRouteDialog
        routeId={deleteTarget}
        onClose={() => setDeleteTarget(null)}
      />
    </div>
  );
}

export function AddRouteDialog({
  open,
  onOpenChange,
  botId,
  ownerOrgId,
  ownerLabel,
  isTelegramManager,
}: {
  readonly open: boolean;
  readonly onOpenChange: (open: boolean) => void;
  readonly botId: string;
  /** Bot's owner scope. When non-null, the conversation is created under
   *  that org and only org-owned agent keys from the same org are shown. */
  readonly ownerOrgId: string | null;
  readonly ownerLabel: string;
  readonly isTelegramManager: boolean;
}) {
  useUiTranslation();
  const { data: apiKeys, isLoading: keysLoading, error: keysError, refetch: refetchKeys } = useApiKeys({ orgId: ownerOrgId });
  const createConversation = useCreateChannelConversation();

  const {
    register,
    handleSubmit,
    setValue,
    setError,
    watch,
    reset,
    formState: { errors },
  } = useAppForm<CreateChannelConversationFormData>({
    resolver: zodResolver(createChannelConversationSchema),
    defaultValues: {
      channel_bot_id: botId,
      agent_api_key_id: "",
      platform_conversation_id: undefined,
      platform_conversation_type: undefined,
      platform_sender_id: undefined,
      default_agent: false,
      target_org_id: ownerOrgId ?? undefined,
    },
  });

  // The dialog component stays mounted across open/close, so a cancelled
  // draft would otherwise leak into the next open.
  function handleOpenChange(next: boolean) {
    if (!next) reset();
    onOpenChange(next);
  }

  function onSubmit(data: CreateChannelConversationFormData) {
    const conversationId = data.platform_conversation_id?.trim();
    if (
      isTelegramManager &&
      !data.default_agent &&
      (!conversationId || conversationId === "*")
    ) {
      setError("platform_conversation_id", {
        message:
          "Enter an exact Telegram chat ID or enable the public default route.",
      });
      return;
    }
    // Backend enforces that channel_bot.user_id, agent_api_key.user_id
    // and target_org_id all match. The dialog is already scoped to the
    // bot's owner so just forward the ownerOrgId explicitly.
    createConversation.mutate(
      {
        ...data,
        ...(isTelegramManager
          ? {
              platform_conversation_id: data.default_agent
                ? undefined
                : conversationId,
              platform_conversation_type: data.default_agent
                ? undefined
                : data.platform_conversation_type,
              platform_sender_id: undefined,
            }
          : {}),
        target_org_id: ownerOrgId ?? undefined,
      },
      {
        onSuccess: () => {
          toast.success(translateUi("Conversation route added"));
          reset();
          onOpenChange(false);
        },
        onError: (err) => {
          const message =
            err instanceof ApiError
              ? err.message
              : "Failed to add conversation route";
          toast.error(translateErrorMessage(message));
        },
      },
    );
  }

  const eligibleKeys = (apiKeys ?? []).filter((key) => !routeAgentIssue(key, ownerOrgId));

  return (
    <Dialog open={open} onOpenChange={handleOpenChange}>
      <DialogContent className="md:max-w-md">
        <DialogHeader>
          <DialogTitle>{translateUi("Add Conversation Route")}</DialogTitle>
          <DialogDescription>{translateUi("Route a conversation to an AI agent using one of your API keys.")}</DialogDescription>
        </DialogHeader>

        <form onSubmit={handleSubmit(onSubmit)} className="space-y-4">
          <input type="hidden" {...register("channel_bot_id")} />

          {isTelegramManager && (
            <div className="space-y-2">
              <div className="flex items-center gap-3">
                <Switch
                  id="public_default_route"
                  checked={watch("default_agent") ?? false}
                  onCheckedChange={(value) => setValue("default_agent", value)}
                />
                <Label htmlFor="public_default_route">{translateUi("Use a public default route")}</Label>
              </div>
              <p className="text-13 text-muted-foreground">
                {watch("default_agent")
                  ? translateUi("Receives ordinary messages from anyone contacting this bot when no exact route matches. Choose a dedicated agent with restricted permissions for public use.")
                  : translateUi("Enter an exact Telegram chat ID to route only that conversation. Turn on the public default route to handle unmatched chats.")}
              </p>
            </div>
          )}

          {keysError ? <ErrorBanner message={translateUi("Agent keys could not be loaded for this owner.")} onRetry={() => void refetchKeys()} /> : keysLoading ? <p className="text-13 text-muted-foreground">{translateUi("Loading agent keys…")}</p> : <RouteReadiness keys={apiKeys ?? []} ownerOrgId={ownerOrgId} ownerLabel={ownerLabel} />}

          <div className="space-y-2">
            <Label htmlFor="agent_api_key_id">{translateUi("Agent (API Key)")}</Label>
            {(
              <Select
                disabled={keysLoading || Boolean(keysError) || eligibleKeys.length === 0}
                value={watch("agent_api_key_id") ?? ""}
                onValueChange={(value) => setValue("agent_api_key_id", value)}
              >
                <SelectTrigger id="agent_api_key_id">
                  <SelectValue placeholder={translateUi("Select an API key")} />
                </SelectTrigger>
                <SelectContent>
                  {(apiKeys ?? []).map((key) => (
                    <SelectItem
                      key={key.id}
                      value={key.id}
                      disabled={Boolean(routeAgentIssue(key, ownerOrgId))}
                    >
                      {key.name}
                      {key.platform ? translateUi(" ({{platform}})", { platform: key.platform }) : ""}
                      {routeAgentIssue(key, ownerOrgId) ? ` — ${routeAgentIssue(key, ownerOrgId)}` : ""}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            )}
            {errors.agent_api_key_id && (
              <p className="text-13 text-destructive">
                {errors.agent_api_key_id.message}
              </p>
            )}
          </div>

          {(!isTelegramManager || !watch("default_agent")) && (
            <>
              <div className="space-y-2">
                <Label htmlFor="platform_conversation_id">
                  {isTelegramManager
                    ? translateUi("Telegram chat ID")
                    : translateUi("Conversation ID (optional)")}
                </Label>
                <Input
                  id="platform_conversation_id"
                  aria-required={isTelegramManager || undefined}
                  placeholder={
                    isTelegramManager
                      ? translateUi("Exact Telegram chat ID")
                      : translateUi("Leave empty for default route")
                  }
                  {...register("platform_conversation_id")}
                />
                <p className="text-13 text-muted-foreground">
                  {isTelegramManager
                    ? translateUi("Only messages from this chat will use this route.")
                    : translateUi("Platform-specific chat/channel ID. Leave empty to create a default route for all unmatched conversations.")}
                </p>
                {errors.platform_conversation_id && (
                  <p className="text-13 text-destructive">
                    {errors.platform_conversation_id.message}
                  </p>
                )}
              </div>

              <div className="space-y-2">
                <Label htmlFor="platform_conversation_type">{translateUi("Conversation Type")}</Label>
                <Select
                  value={watch("platform_conversation_type") ?? ""}
                  onValueChange={(value) =>
                    setValue(
                      "platform_conversation_type",
                      value as "private" | "group" | "channel",
                    )
                  }
                >
                  <SelectTrigger id="platform_conversation_type">
                    <SelectValue placeholder={translateUi("Select type (optional)")} />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="private">{translateUi("Private")}</SelectItem>
                    <SelectItem value="group">{translateUi("Group")}</SelectItem>
                    <SelectItem value="channel">{translateUi("Channel")}</SelectItem>
                  </SelectContent>
                </Select>
              </div>
            </>
          )}

          {!isTelegramManager && (
            <div className="flex items-center gap-3">
              <Switch
                id="default_agent"
                checked={watch("default_agent") ?? false}
                onCheckedChange={(v) => setValue("default_agent", v)}
              />
              <Label htmlFor="default_agent">{translateUi("Set as default agent for this bot")}</Label>
            </div>
          )}

          <DialogFooter>
            <Button
              type="button"
              variant="outline"
              onClick={() => handleOpenChange(false)}
            >{translateUi("Cancel")}</Button>
            <Button
              variant="primary"
              type="submit"
              disabled={
                createConversation.isPending ||
                Boolean(keysError) ||
                !eligibleKeys.some((key) => key.id === watch("agent_api_key_id")) ||
                (isTelegramManager &&
                  !watch("default_agent") &&
                  (!watch("platform_conversation_id")?.trim() ||
                    watch("platform_conversation_id")?.trim() === "*"))
              }
            >
              {createConversation.isPending ? translateUi("Adding...") : translateUi("Add Route")}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}

function DeleteRouteDialog({
  routeId,
  onClose,
}: {
  readonly routeId: string | null;
  readonly onClose: () => void;
}) {
  useUiTranslation();
  const deleteMutation = useDeleteChannelConversation();

  async function handleDelete() {
    if (!routeId) return;
    try {
      await deleteMutation.mutateAsync(routeId);
      toast.success(translateUi("Conversation route removed"));
    } catch (err) {
      toast.error(
        translateErrorMessage(err instanceof ApiError
          ? err.message
          : "Failed to remove conversation route"),
      );
    } finally {
      onClose();
    }
  }

  return (
    <Dialog open={routeId !== null} onOpenChange={() => onClose()}>
      <DialogContent className="md:max-w-md">
        <DialogHeader>
          <DialogTitle>{translateUi("Remove Conversation Route")}</DialogTitle>
          <DialogDescription>{translateUi("This will remove the routing for this conversation. Messages will no longer be relayed to the assigned agent.")}</DialogDescription>
        </DialogHeader>
        <DialogFooter>
          <Button variant="outline" onClick={onClose}>{translateUi("Cancel")}</Button>
          <Button
            variant="destructive"
            onClick={() => void handleDelete()}
            disabled={deleteMutation.isPending}
          >
            {deleteMutation.isPending ? translateUi("Removing...") : translateUi("Remove")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

function DeleteBotDialog({
  open,
  onOpenChange,
  onConfirm,
  isPending,
  deletionNote,
}: {
  readonly open: boolean;
  readonly onOpenChange: (open: boolean) => void;
  readonly onConfirm: () => void;
  readonly isPending: boolean;
  readonly deletionNote?: string;
}) {
  useUiTranslation();
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="md:max-w-md">
        <DialogHeader>
          <DialogTitle>{translateUi("Delete Channel Bot")}</DialogTitle>
          <DialogDescription>{translateUi("This deletes the NyxID connection and its conversation routes. The bot remains on the messaging platform. Reconnecting requires assigning its agents again.")}{deletionNote && translateUi(" {{deletionNote}}", { deletionNote: deletionNote })}
          </DialogDescription>
        </DialogHeader>
        <DialogFooter>
          <Button variant="outline" onClick={() => onOpenChange(false)}>{translateUi("Cancel")}</Button>
          <Button
            variant="destructive"
            onClick={onConfirm}
            disabled={isPending}
          >
            {isPending ? translateUi("Deleting...") : translateUi("Delete")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

/**
 * Renders the Lark / Feishu developer-console permission deep link the
 * backend attaches to `permission_setup_url`. Clicking it lands the user
 * on their app's "Permissions & Scopes" page with the scopes NyxID's
 * adapter requires already pre-checked, ready for "Bulk Enable".
 *
 * Renders nothing when the backend didn't return a URL (non-Lark bots,
 * or Lark bots without an `app_id` configured) so the section
 * disappears cleanly outside the supported flows.
 */
function LarkPermissionSetupSection({
  bot,
}: {
  readonly bot: ChannelBotDetail;
}) {
  useUiTranslation();
  if (!bot.permission_setup_url) {
    return null;
  }
  const scopes = bot.permission_setup_scopes ?? [];

  return (
    <DetailSection title={translateUi("Configure Permissions")}>
      <div className="p-4">
        <p className="text-14 text-muted-foreground">{translateUi("Open this link to grant the scopes NyxID's adapter needs in the Lark/Feishu developer console. The required scopes are pre-selected — confirm and bulk-enable them to finish setup.")}</p>
        {scopes.length > 0 && (
          <div className="mt-3">
            <p className="type-secondary-emphasis text-text-tertiary uppercase tracking-wide">{translateUi("Scopes pre-selected")}</p>
            <ul className="mt-2 flex flex-wrap gap-2">
              {scopes.map((scope) => (
                <li key={scope}>
                  <Badge variant="secondary" className="text-12">
                    {scope}
                  </Badge>
                </li>
              ))}
            </ul>
          </div>
        )}
        <div className="mt-4">
          <Button variant="primary" asChild>
            <a
              href={bot.permission_setup_url}
              target="_blank"
              rel="noopener noreferrer"
            >{translateUi("Open Permissions Page")}<ButtonIcon variant="primary"><ExternalLink className="h-3 w-3" /></ButtonIcon>
            </a>
          </Button>
        </div>
      </div>
    </DetailSection>
  );
}

function TelegramNewSetupSection({ bot }: { readonly bot: ChannelBotDetail }) {
  useUiTranslation();
  return <DetailSection title={translateUi("Telegram connection")}>
    <DetailRow label={translateUi("Webhook")} value={bot.webhook_registered ? "Connected" : "Setup pending"} />
    <p className="p-4 text-13 text-muted-foreground">{bot.status === "suspended" ? translateUi("Telegram reported a management change. Messages and replies are stopped. To keep this bot, delete this NyxID connection and reconnect using the Telegram bot token option with its current token. Deleting the connection removes its NyxID routes; recreate the agent assignments afterward. You can also create a different bot with the Telegram option.") : translateUi("NyxID manages this bot's token and webhook. Assign an agent and send the bot a test message.")}</p>
    {bot.status === "pending" && <div className="p-4"><Button variant="outline" asChild><a href="/channel-bots?connect=telegram-new">{translateUi("Continue Telegram setup")}</a></Button></div>}
  </DetailSection>;
}

function EditBotNameDialog({
  bot,
  onOpenChange,
}: {
  readonly bot: ChannelBotDetail;
  readonly onOpenChange: (open: boolean) => void;
}) {
  useUiTranslation();
  const updateBot = useUpdateChannelBot();
  const {
    register,
    handleSubmit,
    formState: { errors, isDirty, isValid },
  } = useAppForm<UpdateChannelBotFormData>({
    mode: "onChange",
    resolver: zodResolver(updateChannelBotSchema),
    defaultValues: { label: bot.label },
  });

  function onSubmit(data: UpdateChannelBotFormData) {
    const label = data.label?.trim();
    if (!label || updateBot.isPending) return;
    if (label === bot.label) {
      onOpenChange(false);
      return;
    }
    updateBot.mutate({ id: bot.id, data: { label } }, {
      onSuccess: () => {
        toast.success(translateUi("Bot name updated"));
        onOpenChange(false);
      },
      onError: (err) => toast.error(
        translateErrorMessage(err instanceof ApiError ? err.message : "Failed to update bot name"),
      ),
    });
  }

  return (
    <Dialog open onOpenChange={(open) => {
      if (!updateBot.isPending) onOpenChange(open);
    }}>
      <DialogContent className="md:max-w-md">
        <DialogHeader>
          <DialogTitle>{translateUi("Edit bot name")}</DialogTitle>
          <DialogDescription>{translateUi("Choose a name to identify this bot in NyxID.")}</DialogDescription>
        </DialogHeader>
        <form onSubmit={handleSubmit(onSubmit)} className="space-y-4">
          <div className="space-y-2">
            <Label htmlFor="bot-name">{translateUi("Bot name")}</Label>
            <Input
              id="bot-name"
              maxLength={128}
              disabled={updateBot.isPending}
              aria-invalid={Boolean(errors.label)}
              aria-describedby={errors.label ? "bot-name-error" : undefined}
              {...register("label")}
            />
            {errors.label && (
              <p id="bot-name-error" role="alert" className="text-13 text-destructive">
                {errors.label.message}
              </p>
            )}
          </div>
          <DialogFooter>
            <Button type="button" variant="outline" disabled={updateBot.isPending} onClick={() => onOpenChange(false)}>{translateUi("Cancel")}</Button>
            <Button type="submit" variant="primary" disabled={!isDirty || !isValid || updateBot.isPending} isLoading={updateBot.isPending}>{translateUi("Save name")}</Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}

function EditVerificationSection({
  bot,
}: {
  readonly bot: ChannelBotDetail;
}) {
  useUiTranslation();
  const { getPlatform } = useChannelPlatformViews();
  const botId = bot.id;
  const updateBot = useUpdateChannelBot();
  const fields = editableChannelFields(getPlatform(bot.platform));
  const { register, handleSubmit, reset, formState: { errors, isDirty } } = useAppForm<UpdateChannelBotFormData>({
    resolver: zodResolver(updateChannelBotSchema),
    defaultValues: { bot_token: "", verification_token: "", encrypt_key: "", app_id: "", app_secret: "" },
  });

  useEffect(() => {
    reset({ bot_token: "", verification_token: "", encrypt_key: "", app_id: "", app_secret: "" });
  }, [botId, reset]);

  function onSubmit(data: UpdateChannelBotFormData) {
    const payload = Object.fromEntries(fields.flatMap(({ name }) => {
      const value = data[name]?.trim();
      return value ? [[name, value]] : [];
    }));
    if (Object.keys(payload).length === 0) {
      toast.error(translateErrorMessage("Enter at least one value to update"));
      return;
    }
    updateBot.mutate({ id: botId, data: payload }, {
      onSuccess: () => {
        toast.success(translateUi("Credentials updated"));
        reset();
        updateBot.reset();
      },
      onError: (err) => toast.error(translateErrorMessage(err instanceof ApiError ? err.message : "Failed to update credentials")),
    });
  }

  return (
    <DetailSection title={translateUi("Edit Credentials")}>
      <form onSubmit={handleSubmit(onSubmit)} className="space-y-4 p-5">
        {fields.map((field) => (
          <div key={field.name} className="space-y-2">
            <div className="flex items-center justify-between gap-3">
              <Label htmlFor={field.name}>{field.label}</Label>
              {field.configuredKey && bot[field.configuredKey] && <Badge variant="secondary">{translateUi("Configured")}</Badge>}
            </div>
            <Input id={field.name} type={field.secret ? "password" : "text"} autoComplete={field.secret ? "new-password" : "off"}
              placeholder={translateUi("Leave blank to keep current value")} {...register(field.name)} />
            {field.hint && <p className="text-13 text-muted-foreground">{field.hint}</p>}
            {errors[field.name] && <p className="text-13 text-destructive">{errors[field.name]?.message}</p>}
          </div>
        ))}
        {bot.platform === "whatsapp" ? (
          <p className="text-13 text-muted-foreground">{translateUi("Saving stores these credentials immediately. Run Verify Bot to check the saved credentials, then confirm a new message exchange with your agent.")}</p>
        ) : (bot.status === "pending" || bot.status === "pending_webhook") && (
          <p className="text-13 text-muted-foreground">{translateUi("Saving stores these credentials immediately (watch the Configured badges above). The bot stays pending until the platform delivers its first verified webhook, which changes its status to Active.")}</p>
        )}
        <div className="flex justify-end">
          <Button variant="primary" type="submit" disabled={updateBot.isPending || !isDirty} isLoading={updateBot.isPending}>{translateUi("Save Credentials")}</Button>
        </div>
      </form>
    </DetailSection>
  );
}

type ChecklistStatus = "done" | "todo" | "waiting" | "optional";

interface ChecklistRow {
  readonly status: ChecklistStatus;
  readonly label: string;
  readonly hint?: string;
}

function ChecklistItem({ row }: { readonly row: ChecklistRow }) {
  useUiTranslation();
  const icon =
    row.status === "done" ? (
      <Check className="h-3.5 w-3.5 text-success" aria-hidden />
    ) : row.status === "waiting" ? (
      <Loader2
        className="h-3.5 w-3.5 animate-spin text-muted-foreground"
        aria-hidden
      />
    ) : row.status === "todo" ? (
      <Circle className="h-3.5 w-3.5 text-warning" aria-hidden />
    ) : (
      <Circle className="h-3.5 w-3.5 text-muted-foreground/50" aria-hidden />
    );

  return (
    <li className="flex items-start gap-2">
      <span className="mt-0.5 shrink-0">{icon}</span>
      <span className="flex flex-col gap-0.5">
        <span
          className={cn(
            "text-14",
            row.status === "done"
              ? "text-muted-foreground line-through"
              : "text-foreground",
          )}
        >
          {row.label}
        </span>
        {row.hint ? (
          <span className="text-13 text-muted-foreground">{row.hint}</span>
        ) : null}
      </span>
    </li>
  );
}

/**
 * Replaces the passive `pending_webhook` banner with an active setup
 * checklist: webhook URL up top via `CopyableUrlCallout`, then row-per-
 * requirement status. The bot moves to Active automatically on the next
 * verified inbound, so the final row is a `waiting` row that resolves
 * itself once the platform delivers a message.
 *
 * Wave B item B.1.
 */
export function WebhookSetupChecklist({ bot }: { readonly bot: ChannelBotDetail }) {
  useUiTranslation();
  const { getPlatform } = useChannelPlatformViews();
  const { data: runtimeConfig } = useRuntimeConfig();
  // `runtimeConfig.api_base_url` is the authoritative public base for the
  // backend (set in the deploy env). Falls back to the current window
  // origin during the brief moment the runtime-config query is loading,
  // which is also the right default for single-origin local dev where
  // BACKEND_URL is unset.
  const apiBase = runtimeConfig?.api_base_url ?? window.location.origin;
  const webhookUrl = bot.webhook_url ?? `${apiBase}/api/v1/webhooks/channel/${bot.platform}/${bot.id}`;
  const isLark = bot.platform === "lark" || bot.platform === "feishu";
  const isDiscord = bot.platform === "discord";

  const rows: ChecklistRow[] = [];

  if (isLark) {
    rows.push({
      status: bot.lark_verification_token_configured ? "done" : "todo",
      label: "Verification token",
      hint: bot.lark_verification_token_configured
        ? "Configured."
        : "Paste it under Edit Verification below — found in your Lark/Feishu console under Event Subscriptions → Security.",
    });
    rows.push({
      status: bot.lark_encrypt_key_configured ? "done" : "optional",
      label: "Encrypt key",
      hint: bot.lark_encrypt_key_configured
        ? "Configured."
        : "Optional — only required when Encrypt is enabled in the Lark/Feishu console.",
    });
    rows.push({
      status: bot.last_verification?.status === "verified" || bot.app_secret_configured ? "done" : "waiting",
      label: "App ID + App Secret",
      hint: bot.last_verification?.status === "verified"
        ? "App credentials verified. Callback and member verification are separate."
        : bot.app_secret_configured ? "Credentials configured; use Verify Bot to check them."
          : "Use Verify Bot to check the saved app credentials. If credentials are missing, set them under Edit Verification below.",
    });
    if (bot.permission_setup_url) {
      rows.push({
        status: "waiting",
        label: "Bot permissions",
        hint: "Not inspected by NyxID. Review the recommended scopes in the Permissions section below.",
      });
    }
  } else {
    rows.push({
      status: "done",
      label: "Verification",
      hint: bot.webhook_secret_label
        ? `Use the ${bot.webhook_secret_label} shown once at creation in the platform dashboard.`
        : "NyxID verifies incoming requests using the credentials supplied during setup.",
    });
  }

  rows.push({
    status: "waiting",
    label: "Inbound test",
    hint: `Send a test message to the bot in ${getPlatform(bot.platform).label}. Once a verified inbound arrives, this bot moves to Active automatically.`,
  });

  return (
    <div className="space-y-4 rounded-xl border border-amber-500/30 bg-amber-500/10 p-6">
      <div className="space-y-1">
        <p className="type-card-title text-foreground">
          {isDiscord ? translateUi("Finish Discord setup") : isLark ? translateUi("Finish Lark / Feishu setup") : translateUi("Finish webhook setup")}
        </p>
        <p className="text-14 text-muted-foreground">
          {isDiscord ? translateUi("Ordinary chat arrives through the Gateway connection. Send the bot a DM or @mention it; the first authenticated message activates the bot. The Interactions Endpoint URL is optional and only needed for slash commands/components.") : <>{translateUi("Register the URL below in ")}{getPlatform(bot.platform).label}{translateUi(" and complete the checklist. The bot moves to Active automatically once")}{" "}
          {getPlatform(bot.platform).label}{translateUi(" delivers a verified inbound message.")}</>}
          {isLark ? translateUi(" The URL challenge checks callback reachability only; it does not activate the bot or verify a member. Review Setup status below for the separate steps.") : null}
        </p>
      </div>

      <CopyableUrlCallout
        label={isDiscord ? translateUi("Interactions Endpoint URL (optional)") : translateUi("Webhook URL")}
        url={webhookUrl}
        description={isDiscord ? translateUi("For slash commands/components, paste this into Discord Developer Portal > General Information > Interactions Endpoint URL. Do not use Event Webhooks.") : translateUi("Paste this into {{value1}}'s Event Subscriptions / webhook settings.", { value1: getPlatform(bot.platform).label })}
        docsHref={getPlatform(bot.platform).webhookDocs}
      />

      <ul className="space-y-2 pt-1">
        {rows.map((row) => (
          <ChecklistItem key={row.label} row={row} />
        ))}
      </ul>
    </div>
  );
}

export function ChannelBotDetailPage() {
  useUiTranslation();
  const { getPlatform } = useChannelPlatformViews();
  const { botId } = useParams({ strict: false }) as { botId: string };
  const navigate = useNavigate();
  const currentUserId = useAuthStore((s) => s.user?.id ?? null);
  const { data: orgs } = useOrgs();

  const { data: bot, isLoading, error, refetch } = useChannelBot(botId);
  const activityTypes = getPlatform(bot?.platform ?? "").activities;
  const activitySummary = useChannelActivities("bot", botId, activityTypes.length > 0);

  // The bot's owner is an org when `bot.user_id` doesn't match the
  // current user's id. We pass that org id (a user_id in the backend
  // schema) down so child components query and create in the right
  // scope -- the backend enforces `bot.user_id == conversation.user_id
  // == api_key.user_id` for all three resources.
  const ownerOrgId = bot && currentUserId && bot.user_id !== currentUserId
    ? bot.user_id
    : null;

  const { data: apiKeys } = useApiKeys({ orgId: ownerOrgId });
  const deleteMutation = useDeleteChannelBot();
  const verifyMutation = useVerifyChannelBot({ creditsOwnerId: bot?.user_id });

  useBreadcrumbLabel(bot?.label);

  const [showDeleteDialog, setShowDeleteDialog] = useState(false);
  const [showEditNameDialog, setShowEditNameDialog] = useState(false);
  const [verificationContext, setVerificationContext] = useState<{ id: string; updatedAt: string; previousCheckId: string | null } | null>(null);

  const apiKeyNames: ReadonlyMap<string, string> = new Map(
    (apiKeys ?? []).map((k) => [k.id, k.name]),
  );

  const ownerOrg = ownerOrgId
    ? (orgs ?? []).find((o) => o.id === ownerOrgId)
    : undefined;
  const ownerLabel = ownerOrgId
    ? `Org: ${ownerOrg?.display_name ?? ownerOrgId}`
    : "Personal";

  async function handleDelete() {
    try {
      const result = await deleteMutation.mutateAsync(botId);
      if (result?.webhook_cleanup === "failed") {
        toast.warning(translateUi("Bot deleted. Remove its remaining email subscription in the Aurinko dashboard."));
      } else {
        toast.success(translateUi("Bot deleted"));
      }
      void navigate({ to: "/channel-bots" });
    } catch (err) {
      toast.error(
        translateErrorMessage(err instanceof ApiError ? err.message : "Failed to delete bot"),
      );
    } finally {
      setShowDeleteDialog(false);
    }
  }

  async function handleVerify() {
    if (!bot) return;
    setVerificationContext({ id: bot.id, updatedAt: bot.updated_at, previousCheckId: bot.last_verification?.id ?? null });
    try {
      const result = await verifyMutation.mutateAsync(botId);
      if (result.last_verification?.status === "verified") toast.success(translateUi(bot.platform === "lark" || bot.platform === "feishu" ? LARK_VERIFY_SUCCESS : "WhatsApp credentials verified"));
      else if (result.status === "failed" || result.status === "invalid") toast.error(translateErrorMessage("Bot verification failed"));
      else if (bot.platform === "lark" || bot.platform === "feishu") toast.success(translateUi(LARK_VERIFY_SUCCESS));
      else if (result.status === "active") toast.success(translateUi("Bot verified successfully"));
      else toast.success(translateUi("Bot verification completed"));
    } catch (err) {
      toast.error(
        translateErrorMessage(bot.platform === "whatsapp" ? verificationErrorMessage(err) : err instanceof ApiError ? err.message : "Failed to verify bot"),
      );
    }
  }

  if (isLoading) {
    return (
      <div className="space-y-8">
        <Skeleton className="h-12 w-64" />
        <Skeleton className="h-48 w-full" />
      </div>
    );
  }

  if (error || !bot) {
    return (
      <div className="space-y-8">
        <PageHeader title={translateUi("Bot Not Found")} />
        <ErrorBanner
          message={
            error instanceof ApiError
              ? error.message
              : translateUi("Bot not found or failed to load.")
          }
          onRetry={refetch}
        />
      </div>
    );
  }

  return (
    <div className="space-y-8">
      <PageHeader
        title={bot.label}
        description={translateUi("Manage bot settings and conversation routes.")}
        actions={
          <div className="flex gap-2">
            <Button
              variant="outline"
              className="text-text-tertiary hover:text-muted-foreground"
              onClick={() => void handleVerify()}
              disabled={verifyMutation.isPending}
            >
              <ButtonIcon><ShieldCheck className="h-3 w-3" /></ButtonIcon>
              {verifyMutation.isPending ? translateUi("Verifying...") : translateUi("Verify Bot")}
            </Button>
            <Button
              variant="destructive"
              onClick={() => setShowDeleteDialog(true)}
            >
              <ButtonIcon variant="destructive"><Trash2 className="h-3 w-3 text-destructive" /></ButtonIcon>{translateUi("Delete")}</Button>
          </div>
        }
      />

      {bot.platform === "whatsapp" && <CredentialVerification
        check={bot.last_verification}
        pending={verificationContext?.id === bot.id && verifyMutation.isPending}
        error={verificationContext?.id === bot.id && verificationContext.updatedAt === bot.updated_at && verifyMutation.isError ? verificationErrorMessage(verifyMutation.error) : null}
        previousCheckId={verificationContext?.previousCheckId ?? null}
      />}

      {bot.platform !== "whatsapp" && (verifyMutation.variables === botId && verifyMutation.isError ? (
        <div role="alert"><ErrorBanner message={verifyMutation.error instanceof ApiError ? verifyMutation.error.message : translateUi("Failed to verify bot")} /></div>
      ) : bot.credential_source === "telegram_manager" && bot.error ? (
        <div role="alert"><ErrorBanner message={bot.error} /></div>
      ) : verifyMutation.variables === botId && verifyMutation.isSuccess && (
        <p role="status" className="text-13 text-muted-foreground">
          {bot.platform === "lark" || bot.platform === "feishu"
            ? verifyMutation.data.last_verification?.status === "verified" ? LARK_VERIFY_SUCCESS : translateUi("Credential check finished. Review setup status below; callback and member verification are separate.")
            : translateUi("Verification complete. Status: {{value1}}.", { value1: statusLabel(verifyMutation.data.status) })}
        </p>
      ))}

      {bot.status === "pending_webhook" && (bot.credential_source === "telegram_manager" ? <p role="status" className="text-13 text-muted-foreground">{translateUi("Verify Bot checks the existing manager webhook. Review any setup errors in Admin → Platform Credentials → Telegram — bot creation.")}</p> : bot.credential_source === "platform" ? <p role="status" className="text-13 text-muted-foreground">{bot.managed_setup?.subscription === "failed" || bot.managed_setup?.registration === "failed" ? translateUi("Managed setup needs attention. Review the setup status below.") : translateUi("Waiting for the first verified inbound message.")}</p> : <WebhookSetupChecklist bot={bot} />)}

      {(bot.platform === "lark" || bot.platform === "feishu") && <LarkBotSetupStatus bot={bot} />}

      {/* Bot Information */}
      <DetailSection
        title={translateUi("Bot Information")}
        action={
          <Button variant="ghost" size="sm" onClick={() => setShowEditNameDialog(true)}>
            <Pencil className="mr-1.5 h-3 w-3" aria-hidden="true" />{translateUi("Edit name")}</Button>
        }
      >
        <DetailRow label={translateUi("Bot name")} value={bot.label} />
        <DetailRow label={translateUi("Credential source")} value={bot.credential_source === "telegram_manager" ? "Telegram manager" : bot.credential_source === "connection" ? getPlatform(bot.platform).connectedLabel ?? "Connected account" : bot.credential_source === "platform" ? "Platform-managed" : "Your own app"} />
        <DetailRow
          label={translateUi("Platform")}
          value={getPlatform(bot.platform).label}
          badge
          badgeVariant="secondary"
        />
        <DetailRow label={translateUi("Bot Username")} value={bot.platform_bot_username || "-"} />
        <DetailRow label={getPlatform(bot.platform).identityLabel ?? translateUi("Platform Bot ID")} value={bot.platform_bot_id || "-"} copyable />
        {getPlatform(bot.platform).detailFields?.map(({ name, label }) => typeof bot[name] === "string" && bot[name] ? <DetailRow key={name} label={label} value={String(bot[name])} copyable /> : null)}
        <DetailRow label={bot.platform === "whatsapp" ? translateUi("Webhook lifecycle") : translateUi("Status")} value={statusLabel(bot.status)} badge badgeVariant={statusBadgeVariant(bot.status)} />
        {bot.webhook_ingestion === false ? <DetailRow label={translateUi("Ingestion")} value="Polling" /> : <DetailRow label={translateUi("Webhook")} value={bot.platform === "whatsapp" ? (bot.webhook_registered ? "Previously observed (historical)" : "Not yet observed") : bot.webhook_registered ? "Registered" : "Not registered"} />}
        <DetailRow label={translateUi("Owner")} value={ownerLabel} />
        <DetailRow label={translateUi("Created")} value={formatDate(bot.created_at)} />
        <DetailRow label={translateUi("Updated")} value={formatRelativeTime(bot.updated_at)} />
        <DetailRow
          label={translateUi("Configured routes")}
          value={String(bot.conversations_count)}
        />
        {activityTypes.length > 0 && <DetailRow label={translateUi("Received activities (30 days)")} value={activitySummary.error ? "Unavailable" : activitySummary.data ? String(activitySummary.data.total) : "Loading…"} />}
      </DetailSection>

      {Boolean(bot.setup_instructions?.length) && (
        <DetailSection title={translateUi("{{value1}} Setup", { value1: getPlatform(bot.platform).label })}>
          <div className="space-y-4 p-5">
            <CopyableUrlCallout label={bot.platform === "discord" ? translateUi("Interactions Endpoint URL") : translateUi("Callback URL")} url={bot.webhook_url ?? ""} docsHref={getPlatform(bot.platform).webhookDocs} />
            <ol className="list-decimal space-y-2 pl-4 text-13 text-muted-foreground">
              {bot.setup_instructions?.map((instruction) => <li key={instruction}>{instruction}</li>)}
            </ol>
          </div>
        </DetailSection>
      )}
      {bot.permission_setup_url && <LarkPermissionSetupSection bot={bot} />}
      {bot.platform === "telegram-new" && <TelegramNewSetupSection bot={bot} />}
      {bot.credential_source === "telegram_manager" && (
        <DetailSection title={translateUi("Telegram manager routing")}>
          <div className="space-y-3 p-5 text-13 text-muted-foreground">
            <p>{translateUi("This manager can also act as a public channel bot. Add a public default route for ordinary messages from anyone contacting it, using a dedicated agent with restricted permissions. To handle one chat, add its exact Telegram chat ID with the public default route off. Exact routes take priority over the default.")}</p>
            <p>{translateUi("All manager chat callbacks carry signed callback authentication and a reply token only. They do not carry the owner's X-NyxID-User-Token. Telegram senders are not mapped to NyxID identities, including on exact chat routes.")}</p>
            <p>
              /start, /recover, and all callback queries remain reserved for bot
              creation and recovery. Ordinary chat delivery is capped at 32 concurrent
              requests per backend process. Messages arriving at capacity are dropped.
            </p>
            <p>{translateUi("Rotate the manager token in Admin → Platform Credentials → Telegram — bot creation. Deleting this channel connection keeps the manager and bot creation available.")}</p>
          </div>
        </DetailSection>
      )}
      {(() => {
        if (bot.credential_source === "telegram_manager") return null;
        const flow = getPlatform(bot.platform).managedFlow;
        const ManagedDetail = flow ? MANAGED_FLOW_COMPONENTS[flow].Detail : undefined;
        return bot.credential_source !== "user" && bot.credential_source && ManagedDetail ? <ManagedDetail bot={bot} orgId={ownerOrgId} /> : editableChannelFields(getPlatform(bot.platform)).length > 0 && <EditVerificationSection bot={bot} />;
      })()}

      {/* Conversation Routes */}
      {bot.platform === "whatsapp" && <WhatsappNativeSection botId={bot.id} />}
      <ConversationsSection
        platform={bot.platform}
        botId={botId}
        apiKeyNames={apiKeyNames}
        ownerOrgId={ownerOrgId}
        ownerLabel={ownerLabel}
        isTelegramManager={bot.credential_source === "telegram_manager"}
      />

      {showEditNameDialog && (
        <EditBotNameDialog key={bot.id} bot={bot} onOpenChange={setShowEditNameDialog} />
      )}
      <OwnershipTransferCard
        kind="channel_bot"
        resource={{
          id: bot.id,
          name: bot.label,
          owner_user_id: bot.user_id,
          platform: bot.platform,
          slug: null,
        }}
        onTransferred={() => void navigate({ to: "/channel-bots" })}
      />
      {/* Delete Confirmation */}
      <DeleteBotDialog
        deletionNote={bot.credential_source === "telegram_manager" ? TELEGRAM_MANAGER_DELETION_NOTE : bot.credential_source !== "user" ? getPlatform(bot.platform).deletionNote : undefined}
        open={showDeleteDialog}
        onOpenChange={setShowDeleteDialog}
        onConfirm={() => void handleDelete()}
        isPending={deleteMutation.isPending}
      />
    </div>
  );
}
