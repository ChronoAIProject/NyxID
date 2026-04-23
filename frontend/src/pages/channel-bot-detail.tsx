import { useEffect, useState } from "react";
import { useParams, useNavigate, Link } from "@tanstack/react-router";
import { useForm } from "react-hook-form";
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
import { formatDate, formatRelativeTime } from "@/lib/utils";
import { PageHeader } from "@/components/shared/page-header";
import { DetailSection } from "@/components/shared/detail-section";
import { DetailRow } from "@/components/shared/detail-row";
import { Skeleton } from "@/components/ui/skeleton";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
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
  Bot,
  Check,
  ExternalLink,
  MessageSquare,
  Plus,
  ShieldCheck,
  Trash2,
} from "lucide-react";
import { toast } from "sonner";
import type {
  ChannelBotDetail,
  ChannelBotStatus,
  ChannelConversationItem,
  ChannelPlatform,
} from "@/types/channels";

function statusBadgeVariant(
  status: ChannelBotStatus,
): "success" | "warning" | "destructive" | "secondary" {
  switch (status) {
    case "active":
      return "success";
    case "pending":
    case "pending_webhook":
      return "warning";
    case "failed":
      return "destructive";
    case "invalid":
      return "secondary";
    default:
      return "secondary";
  }
}

function platformLabel(platform: ChannelPlatform): string {
  switch (platform) {
    case "telegram":
      return "Telegram";
    case "discord":
      return "Discord";
    case "lark":
      return "Lark";
    case "feishu":
      return "Feishu";
    case "slack":
      return "Slack";
    default:
      return platform;
  }
}

function conversationTypeLabel(t: string): string {
  switch (t) {
    case "private":
      return "Private";
    case "group":
      return "Group";
    case "channel":
      return "Channel";
    default:
      return t;
  }
}

function ConversationRow({
  conversation,
  apiKeyNames,
  botId,
  onDelete,
}: {
  readonly conversation: ChannelConversationItem;
  readonly apiKeyNames: ReadonlyMap<string, string>;
  readonly botId: string;
  readonly onDelete: (id: string) => void;
}) {
  const agentName =
    apiKeyNames.get(conversation.agent_api_key_id) ??
    conversation.agent_api_key_id.slice(0, 8);

  return (
    <TableRow>
      <TableCell className="font-mono text-xs">
        {conversation.platform_conversation_id ||
          conversation.platform_sender_id ||
          "-"}
      </TableCell>
      <TableCell>
        <Badge variant="outline">
          {conversationTypeLabel(conversation.platform_conversation_type)}
        </Badge>
      </TableCell>
      <TableCell className="font-medium">{agentName}</TableCell>
      <TableCell>
        {conversation.default_agent ? (
          <Badge variant="success">Default</Badge>
        ) : (
          <span className="text-xs text-muted-foreground">-</span>
        )}
      </TableCell>
      <TableCell>
        {conversation.is_active ? (
          <Badge variant="success">Active</Badge>
        ) : (
          <Badge variant="secondary">Inactive</Badge>
        )}
      </TableCell>
      <TableCell className="text-xs text-muted-foreground">
        {conversation.last_message_at
          ? formatRelativeTime(conversation.last_message_at)
          : "Never"}
      </TableCell>
      <TableCell className="w-[100px]">
        <div className="flex items-center gap-1">
          <Link
            to={`/channel-bots/${botId}/conversations/${conversation.id}` as string}
          >
            <Button variant="ghost" size="icon" className="h-8 w-8">
              <MessageSquare className="h-4 w-4 text-muted-foreground" />
            </Button>
          </Link>
          <Button
            variant="ghost"
            size="icon"
            className="h-8 w-8"
            onClick={() => onDelete(conversation.id)}
          >
            <Trash2 className="h-4 w-4 text-muted-foreground" />
          </Button>
        </div>
      </TableCell>
    </TableRow>
  );
}

function ConversationsSection({
  botId,
  apiKeyNames,
  ownerOrgId,
}: {
  readonly botId: string;
  readonly apiKeyNames: ReadonlyMap<string, string>;
  /** When the parent bot is org-owned, the org id (a user_id). Used to
   *  scope the conversation list and pre-fill `target_org_id` on create.
   *  `null` means personal. */
  readonly ownerOrgId: string | null;
}) {
  const { data: conversations, isLoading } = useChannelConversations({
    botId,
    orgId: ownerOrgId,
  });
  const [addOpen, setAddOpen] = useState(false);
  const [deleteTarget, setDeleteTarget] = useState<string | null>(null);

  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between">
        <div>
          <h3 className="text-lg font-medium">Conversation Routes</h3>
          <p className="text-sm text-muted-foreground">
            Map conversations to AI agents for message relay.
          </p>
        </div>
        <Button
          variant="outline"
          size="sm"
          onClick={() => setAddOpen(true)}
        >
          <Plus className="mr-2 h-4 w-4" />
          Add Route
        </Button>
      </div>

      {isLoading ? (
        <div className="space-y-2">
          {Array.from({ length: 2 }, (_, i) => (
            <Skeleton key={`conv-skel-${String(i)}`} className="h-12 w-full" />
          ))}
        </div>
      ) : !conversations || conversations.length === 0 ? (
        <div className="flex flex-col items-center justify-center rounded-xl border border-border py-8 text-center">
          <MessageSquare className="mb-3 h-8 w-8 text-muted-foreground/50" />
          <p className="text-sm text-muted-foreground">
            No conversation routes configured. Add a route to start relaying
            messages to an AI agent.
          </p>
        </div>
      ) : (
        <div className="rounded-xl border border-border">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>Conversation ID</TableHead>
                <TableHead>Type</TableHead>
                <TableHead>Agent</TableHead>
                <TableHead>Default</TableHead>
                <TableHead>Status</TableHead>
                <TableHead>Last Message</TableHead>
                <TableHead className="w-[100px]" />
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
                />
              ))}
            </TableBody>
          </Table>
        </div>
      )}

      <AddRouteDialog
        open={addOpen}
        onOpenChange={setAddOpen}
        botId={botId}
        ownerOrgId={ownerOrgId}
      />
      <DeleteRouteDialog
        routeId={deleteTarget}
        onClose={() => setDeleteTarget(null)}
      />
    </div>
  );
}

function AddRouteDialog({
  open,
  onOpenChange,
  botId,
  ownerOrgId,
}: {
  readonly open: boolean;
  readonly onOpenChange: (open: boolean) => void;
  readonly botId: string;
  /** Bot's owner scope. When non-null, the conversation is created under
   *  that org and only org-owned agent keys from the same org are shown. */
  readonly ownerOrgId: string | null;
}) {
  const { data: apiKeys } = useApiKeys({ orgId: ownerOrgId });
  const createConversation = useCreateChannelConversation();
  const [defaultAgent, setDefaultAgent] = useState(false);

  const {
    register,
    handleSubmit,
    setValue,
    reset,
    formState: { errors },
  } = useForm<CreateChannelConversationFormData>({
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

  function onSubmit(data: CreateChannelConversationFormData) {
    // Backend enforces that channel_bot.user_id, agent_api_key.user_id
    // and target_org_id all match. The dialog is already scoped to the
    // bot's owner so just forward the ownerOrgId explicitly.
    createConversation.mutate(
      {
        ...data,
        default_agent: defaultAgent,
        target_org_id: ownerOrgId ?? undefined,
      },
      {
        onSuccess: () => {
          toast.success("Conversation route added");
          reset();
          setDefaultAgent(false);
          onOpenChange(false);
        },
        onError: (err) => {
          const message =
            err instanceof ApiError
              ? err.message
              : "Failed to add conversation route";
          toast.error(message);
        },
      },
    );
  }

  const activeApiKeys = (apiKeys ?? []).filter((k) => k.is_active);
  const keysWithCallback = activeApiKeys.filter((k) => k.callback_url);

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-md">
        <DialogHeader>
          <DialogTitle>Add Conversation Route</DialogTitle>
          <DialogDescription>
            Route a conversation to an AI agent using one of your API keys.
          </DialogDescription>
        </DialogHeader>

        <form onSubmit={handleSubmit(onSubmit)} className="space-y-4">
          <input type="hidden" {...register("channel_bot_id")} />

          <div className="space-y-2">
            <Label htmlFor="agent_api_key_id">Agent (API Key)</Label>
            {keysWithCallback.length === 0 && activeApiKeys.length > 0 ? (
              <div className="rounded-md border border-border bg-muted/50 p-3">
                <p className="text-sm text-muted-foreground">
                  None of your agent keys have a callback URL set.
                  Go to{" "}
                  <a href="/keys?tab=nyxid" className="text-primary underline">
                    Agent Keys
                  </a>
                  {" "}and set a Callback URL on the key you want to use as an agent.
                </p>
              </div>
            ) : (
              <Select
                onValueChange={(value) => setValue("agent_api_key_id", value)}
              >
                <SelectTrigger>
                  <SelectValue placeholder="Select an API key" />
                </SelectTrigger>
                <SelectContent>
                  {activeApiKeys.map((key) => (
                    <SelectItem
                      key={key.id}
                      value={key.id}
                      disabled={!key.callback_url}
                    >
                      {key.name}
                      {key.platform ? ` (${key.platform})` : ""}
                      {!key.callback_url ? " -- no callback URL" : ""}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            )}
            {errors.agent_api_key_id && (
              <p className="text-xs text-destructive">
                {errors.agent_api_key_id.message}
              </p>
            )}
          </div>

          <div className="space-y-2">
            <Label htmlFor="platform_conversation_id">
              Conversation ID (optional)
            </Label>
            <Input
              id="platform_conversation_id"
              placeholder="Leave empty for default route"
              {...register("platform_conversation_id")}
            />
            <p className="text-xs text-muted-foreground">
              Platform-specific chat/channel ID. Leave empty to create a default
              route for all unmatched conversations.
            </p>
          </div>

          <div className="space-y-2">
            <Label>Conversation Type</Label>
            <Select
              onValueChange={(value) =>
                setValue(
                  "platform_conversation_type",
                  value as "private" | "group" | "channel",
                )
              }
            >
              <SelectTrigger>
                <SelectValue placeholder="Select type (optional)" />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="private">Private</SelectItem>
                <SelectItem value="group">Group</SelectItem>
                <SelectItem value="channel">Channel</SelectItem>
              </SelectContent>
            </Select>
          </div>

          <div className="flex items-center gap-3">
            <Switch
              id="default_agent"
              checked={defaultAgent}
              onCheckedChange={setDefaultAgent}
            />
            <Label htmlFor="default_agent" className="text-sm">
              Set as default agent for this bot
            </Label>
          </div>

          <DialogFooter>
            <Button
              type="button"
              variant="outline"
              onClick={() => onOpenChange(false)}
            >
              Cancel
            </Button>
            <Button type="submit" disabled={createConversation.isPending}>
              {createConversation.isPending ? "Adding..." : "Add Route"}
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
  const deleteMutation = useDeleteChannelConversation();

  async function handleDelete() {
    if (!routeId) return;
    try {
      await deleteMutation.mutateAsync(routeId);
      toast.success("Conversation route removed");
    } catch (err) {
      toast.error(
        err instanceof ApiError
          ? err.message
          : "Failed to remove conversation route",
      );
    } finally {
      onClose();
    }
  }

  return (
    <Dialog open={routeId !== null} onOpenChange={() => onClose()}>
      <DialogContent className="sm:max-w-md">
        <DialogHeader>
          <DialogTitle>Remove Conversation Route</DialogTitle>
          <DialogDescription>
            This will remove the routing for this conversation. Messages will no
            longer be relayed to the assigned agent.
          </DialogDescription>
        </DialogHeader>
        <DialogFooter>
          <Button variant="outline" onClick={onClose}>
            Cancel
          </Button>
          <Button
            variant="destructive"
            onClick={() => void handleDelete()}
            disabled={deleteMutation.isPending}
          >
            {deleteMutation.isPending ? "Removing..." : "Remove"}
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
}: {
  readonly open: boolean;
  readonly onOpenChange: (open: boolean) => void;
  readonly onConfirm: () => void;
  readonly isPending: boolean;
}) {
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-md">
        <DialogHeader>
          <DialogTitle>Delete Channel Bot</DialogTitle>
          <DialogDescription>
            This will permanently delete this bot and all its conversation
            routes. This action cannot be undone.
          </DialogDescription>
        </DialogHeader>
        <DialogFooter>
          <Button variant="outline" onClick={() => onOpenChange(false)}>
            Cancel
          </Button>
          <Button
            variant="destructive"
            onClick={onConfirm}
            disabled={isPending}
          >
            {isPending ? "Deleting..." : "Delete"}
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
  if (!bot.permission_setup_url) {
    return null;
  }
  const scopes = bot.permission_setup_scopes ?? [];

  return (
    <DetailSection title="Configure Permissions">
      <p className="text-sm text-muted-foreground">
        Open this link to grant the scopes NyxID's adapter needs in the
        Lark/Feishu developer console. The required scopes are
        pre-selected — confirm and bulk-enable them to finish setup.
      </p>
      {scopes.length > 0 && (
        <div className="mt-3">
          <p className="text-xs font-medium text-text-tertiary uppercase tracking-wide">
            Scopes pre-selected
          </p>
          <ul className="mt-2 flex flex-wrap gap-2">
            {scopes.map((scope) => (
              <li key={scope}>
                <Badge variant="outline" className="font-mono text-xs">
                  {scope}
                </Badge>
              </li>
            ))}
          </ul>
        </div>
      )}
      <div className="mt-4">
        <Button asChild size="sm">
          <a
            href={bot.permission_setup_url}
            target="_blank"
            rel="noopener noreferrer"
          >
            Open Permissions Page
            <ExternalLink className="ml-2 h-4 w-4" />
          </a>
        </Button>
      </div>
    </DetailSection>
  );
}

function EditVerificationSection({
  bot,
}: {
  readonly bot: ChannelBotDetail;
}) {
  const botId = bot.id;
  const updateBot = useUpdateChannelBot();
  const {
    register,
    handleSubmit,
    reset,
    formState: { errors, isDirty },
  } = useForm<UpdateChannelBotFormData>({
    resolver: zodResolver(updateChannelBotSchema),
    defaultValues: {
      verification_token: "",
      encrypt_key: "",
      app_id: "",
      app_secret: "",
    },
  });

  useEffect(() => {
    reset({
      verification_token: "",
      encrypt_key: "",
      app_id: "",
      app_secret: "",
    });
  }, [botId, reset]);

  function onSubmit(data: UpdateChannelBotFormData) {
    const payload = {
      verification_token: data.verification_token?.trim() || undefined,
      encrypt_key: data.encrypt_key?.trim() || undefined,
      app_id: data.app_id?.trim() || undefined,
      app_secret: data.app_secret?.trim() || undefined,
    };

    if (
      !payload.verification_token &&
      !payload.encrypt_key &&
      !payload.app_id &&
      !payload.app_secret
    ) {
      toast.error("Enter at least one value to update");
      return;
    }

    updateBot.mutate(
      { id: botId, data: payload },
      {
        onSuccess: () => {
          toast.success("Verification settings updated");
          reset({
            verification_token: "",
            encrypt_key: "",
            app_id: "",
            app_secret: "",
          });
        },
        onError: (err) => {
          toast.error(
            err instanceof ApiError
              ? err.message
              : "Failed to update verification settings",
          );
        },
      },
    );
  }

  return (
    <DetailSection title="Edit Verification">
      <form onSubmit={handleSubmit(onSubmit)} className="space-y-4">
        <div className="space-y-2">
          <div className="flex items-center justify-between gap-3">
            <Label htmlFor="verification_token">Verification Token</Label>
            {bot.lark_verification_token_configured && (
              <Badge variant="outline" className="text-[10px] uppercase tracking-wide">
                Configured
              </Badge>
            )}
          </div>
          <Input
            id="verification_token"
            type="password"
            placeholder="Paste the Event Subscriptions Verification Token"
            {...register("verification_token")}
          />
          <p className="text-xs text-muted-foreground">
            Required by Lark/Feishu webhook verification. Found in Event
            Subscriptions → Security.
          </p>
          {errors.verification_token && (
            <p className="text-xs text-destructive">
              {errors.verification_token.message}
            </p>
          )}
        </div>

        <div className="space-y-2">
          <div className="flex items-center justify-between gap-3">
            <Label htmlFor="encrypt_key">Encrypt Key</Label>
            {bot.lark_encrypt_key_configured && (
              <Badge variant="outline" className="text-[10px] uppercase tracking-wide">
                Configured
              </Badge>
            )}
          </div>
          <Input
            id="encrypt_key"
            type="password"
            placeholder="Optional Encrypt Key from Event Subscriptions"
            {...register("encrypt_key")}
          />
          <p className="text-xs text-muted-foreground">
            Optional. Leave blank to keep the current value. Use the API or CLI
            with an empty `encrypt_key` to clear it explicitly.
          </p>
          {errors.encrypt_key && (
            <p className="text-xs text-destructive">
              {errors.encrypt_key.message}
            </p>
          )}
        </div>

        <div className="space-y-2">
          <Label htmlFor="app_id">App ID</Label>
          <Input
            id="app_id"
            placeholder="cli_xxxxxxxxxx"
            {...register("app_id")}
          />
          {errors.app_id && (
            <p className="text-xs text-destructive">{errors.app_id.message}</p>
          )}
        </div>

        <div className="space-y-2">
          <div className="flex items-center justify-between gap-3">
            <Label htmlFor="app_secret">App Secret</Label>
            {bot.app_secret_configured && (
              <Badge variant="outline" className="text-[10px] uppercase tracking-wide">
                Configured
              </Badge>
            )}
          </div>
          <Input
            id="app_secret"
            type="password"
            placeholder="Paste a new App Secret if needed"
            {...register("app_secret")}
          />
          {errors.app_secret && (
            <p className="text-xs text-destructive">
              {errors.app_secret.message}
            </p>
          )}
        </div>

        <div className="flex justify-end">
          <Button type="submit" disabled={updateBot.isPending || !isDirty}>
            {updateBot.isPending ? "Saving..." : "Save Verification Settings"}
          </Button>
        </div>
      </form>
    </DetailSection>
  );
}

export function ChannelBotDetailPage() {
  const { botId } = useParams({ strict: false }) as { botId: string };
  const navigate = useNavigate();
  const currentUserId = useAuthStore((s) => s.user?.id ?? null);
  const { data: orgs } = useOrgs();

  const { data: bot, isLoading, error } = useChannelBot(botId);

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
  const verifyMutation = useVerifyChannelBot();

  const [showDeleteDialog, setShowDeleteDialog] = useState(false);

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
      await deleteMutation.mutateAsync(botId);
      toast.success("Bot deleted");
      void navigate({ to: "/channel-bots" });
    } catch (err) {
      toast.error(
        err instanceof ApiError ? err.message : "Failed to delete bot",
      );
    } finally {
      setShowDeleteDialog(false);
    }
  }

  async function handleVerify() {
    try {
      await verifyMutation.mutateAsync(botId);
      toast.success("Bot verification initiated");
    } catch (err) {
      toast.error(
        err instanceof ApiError ? err.message : "Failed to verify bot",
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
      <div className="flex flex-col items-center justify-center py-12 text-center">
        <Bot className="mb-4 h-12 w-12 text-muted-foreground/50" />
        <p className="text-sm text-muted-foreground">
          Bot not found or failed to load.
        </p>
        <Button
          variant="outline"
          className="mt-4"
          onClick={() => void navigate({ to: "/channel-bots" })}
        >
          Back to Channel Bots
        </Button>
      </div>
    );
  }

  return (
    <div className="space-y-8">
      <PageHeader
        breadcrumbs={[
          { label: "Channel Bots", to: "/channel-bots" },
          { label: bot.label },
        ]}
        title={bot.label}
        description="Manage bot settings and conversation routes."
        actions={
          <div className="flex gap-2">
            <Button
              variant="outline"
              size="sm"
              onClick={() => void handleVerify()}
              disabled={verifyMutation.isPending}
            >
              <ShieldCheck className="mr-2 h-4 w-4" />
              {verifyMutation.isPending ? "Verifying..." : "Verify Bot"}
            </Button>
            <Button
              variant="destructive"
              size="sm"
              onClick={() => setShowDeleteDialog(true)}
            >
              <Trash2 className="mr-2 h-4 w-4" />
              Delete
            </Button>
          </div>
        }
      />

      {bot.status === "pending_webhook" && (
        <div className="rounded-xl border border-amber-500/30 bg-amber-500/10 p-4">
          <p className="text-sm font-medium text-foreground">
            Pending webhook verification
          </p>
          <p className="mt-1 text-sm text-muted-foreground">
            {bot.platform === "lark" || bot.platform === "feishu"
              ? bot.lark_verification_token_configured
                ? "Once Lark/Feishu delivers a verified inbound message, this bot will automatically move to Active."
                : "Set the Verification Token below, and set Encrypt Key too if it is enabled in the Lark/Feishu console. After the next verified inbound message, this bot will automatically move to Active."
              : `Once ${platformLabel(bot.platform)} delivers a verified inbound message, this bot will automatically move to Active.`}
          </p>
        </div>
      )}

      {/* Bot Information */}
      <DetailSection title="Bot Information">
        <DetailRow
          label="Platform"
          value={platformLabel(bot.platform)}
          badge
          badgeVariant="outline"
        />
        <DetailRow label="Bot Username" value={bot.platform_bot_username || "-"} mono />
        <DetailRow label="Platform Bot ID" value={bot.platform_bot_id || "-"} mono copyable />
        <div className="flex items-center justify-between border-b border-border py-2 text-sm last:border-b-0">
          <span className="text-text-tertiary">Status</span>
          <Badge variant={statusBadgeVariant(bot.status)}>{bot.status}</Badge>
        </div>
        <div className="flex items-center justify-between border-b border-border py-2 text-sm last:border-b-0">
          <span className="text-text-tertiary">Webhook</span>
          {bot.webhook_registered ? (
            <div className="flex items-center gap-1">
              <Check className="h-3 w-3 text-success" />
              <span className="text-foreground">Registered</span>
            </div>
          ) : (
            <span className="text-muted-foreground">Not registered</span>
          )}
        </div>
        <DetailRow label="Owner" value={ownerLabel} />
        <DetailRow label="Created" value={formatDate(bot.created_at)} />
        <DetailRow label="Updated" value={formatRelativeTime(bot.updated_at)} />
        <DetailRow
          label="Conversations"
          value={String(bot.conversations_count)}
        />
      </DetailSection>

      {(bot.platform === "lark" || bot.platform === "feishu") && (
        <>
          <LarkPermissionSetupSection bot={bot} />
          <EditVerificationSection bot={bot} />
        </>
      )}

      {/* Conversation Routes */}
      <ConversationsSection
        botId={botId}
        apiKeyNames={apiKeyNames}
        ownerOrgId={ownerOrgId}
      />

      {/* Delete Confirmation */}
      <DeleteBotDialog
        open={showDeleteDialog}
        onOpenChange={setShowDeleteDialog}
        onConfirm={() => void handleDelete()}
        isPending={deleteMutation.isPending}
      />
    </div>
  );
}
