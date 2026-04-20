import { useEffect, useMemo, useState } from "react";
import { useNavigate } from "@tanstack/react-router";
import { useForm } from "react-hook-form";
import { zodResolver } from "@hookform/resolvers/zod";
import { useChannelBots, useCreateChannelBot, useDeleteChannelBot } from "@/hooks/use-channel-bots";
import {
  useChannelConversations,
  useCreateDeviceConversation,
  useDeleteChannelConversation,
} from "@/hooks/use-channel-conversations";
import { useApiKeys } from "@/hooks/use-api-keys";
import {
  createChannelBotSchema,
  createDeviceConversationSchema,
  type CreateChannelBotFormData,
  type CreateDeviceConversationFormData,
} from "@/schemas/channels";
import { ApiError } from "@/lib/api-client";
import { formatDate } from "@/lib/utils";
import { PageHeader } from "@/components/shared/page-header";
import { OrgScopeSelect } from "@/components/shared/org-scope-select";
import { Skeleton } from "@/components/ui/skeleton";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Card, CardContent } from "@/components/ui/card";
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
import { Bot, Check, Copy, Cpu, Plus, Trash2 } from "lucide-react";
import { toast } from "sonner";
import type {
  ChannelBotItem,
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

function BotRow({
  bot,
  onDelete,
}: {
  readonly bot: ChannelBotItem;
  readonly onDelete: (id: string) => void;
}) {
  const navigate = useNavigate();

  return (
    <TableRow
      className="cursor-pointer"
      onClick={() => void navigate({ to: "/channel-bots/$botId", params: { botId: bot.id } })}
    >
      <TableCell>
        <Badge variant="outline">{platformLabel(bot.platform)}</Badge>
      </TableCell>
      <TableCell className="font-mono text-xs">
        {bot.platform_bot_username || "-"}
      </TableCell>
      <TableCell className="font-medium">{bot.label}</TableCell>
      <TableCell>
        <Badge variant={statusBadgeVariant(bot.status)}>
          {bot.status}
        </Badge>
      </TableCell>
      <TableCell>
        {bot.webhook_registered ? (
          <div className="flex items-center gap-1 text-xs text-muted-foreground">
            <Check className="h-3 w-3 text-success" />
            Registered
          </div>
        ) : (
          <span className="text-xs text-muted-foreground">Not registered</span>
        )}
      </TableCell>
      <TableCell className="text-xs text-muted-foreground">
        {formatDate(bot.created_at)}
      </TableCell>
      <TableCell className="w-[60px]">
        <Button
          variant="ghost"
          size="icon"
          className="h-8 w-8"
          onClick={(e) => {
            e.stopPropagation();
            onDelete(bot.id);
          }}
        >
          <Trash2 className="h-4 w-4 text-muted-foreground" />
        </Button>
      </TableCell>
    </TableRow>
  );
}

function BotsTable({
  bots,
  onDelete,
}: {
  readonly bots: readonly ChannelBotItem[];
  readonly onDelete: (id: string) => void;
}) {
  return (
    <div className="rounded-xl border border-border">
      <Table>
        <TableHeader>
          <TableRow>
            <TableHead>Platform</TableHead>
            <TableHead>Bot Username</TableHead>
            <TableHead>Label</TableHead>
            <TableHead>Status</TableHead>
            <TableHead>Webhook</TableHead>
            <TableHead>Created</TableHead>
            <TableHead className="w-[60px]" />
          </TableRow>
        </TableHeader>
        <TableBody>
          {bots.map((bot) => (
            <BotRow key={bot.id} bot={bot} onDelete={onDelete} />
          ))}
        </TableBody>
      </Table>
    </div>
  );
}

function EmptyState({ onAdd }: { readonly onAdd: () => void }) {
  return (
    <Card>
      <CardContent className="flex flex-col items-center gap-4 py-16">
        <div className="flex h-14 w-14 items-center justify-center rounded-full border border-border">
          <Bot className="h-6 w-6 text-muted-foreground" />
        </div>
        <div className="text-center">
          <p className="text-sm font-medium">No channel bots yet</p>
          <p className="text-xs text-muted-foreground">
            Add a messaging platform bot to relay conversations to your AI agents.
          </p>
        </div>
        <Button size="sm" onClick={onAdd}>
          <Plus className="mr-2 h-4 w-4" />
          Add Bot
        </Button>
      </CardContent>
    </Card>
  );
}

function CreateBotDialog({
  open,
  onOpenChange,
  defaultOrgId,
}: {
  readonly open: boolean;
  readonly onOpenChange: (open: boolean) => void;
  /** Pre-select this org in the scope picker when the page already has one
   *  active. `null` defaults to personal. */
  readonly defaultOrgId: string | null;
}) {
  const createBot = useCreateChannelBot();
  const {
    register,
    handleSubmit,
    watch,
    setValue,
    reset,
    formState: { errors },
  } = useForm<CreateChannelBotFormData>({
    resolver: zodResolver(createChannelBotSchema),
    defaultValues: {
      platform: "telegram",
      bot_token: "",
      label: "",
      target_org_id: defaultOrgId ?? undefined,
    },
  });

  // RHF's `defaultValues` only apply on first mount. The dialog stays
  // mounted across page-scope changes, so re-seed the form whenever the
  // page scope changes OR the dialog (re)opens. Otherwise the dialog
  // would silently submit with the stale first-mount scope -- e.g.
  // switch page scope to an org, click "Add Bot", and it would create a
  // personal bot.
  useEffect(() => {
    if (!open) return;
    reset({
      platform: "telegram",
      bot_token: "",
      label: "",
      target_org_id: defaultOrgId ?? undefined,
    });
  }, [open, defaultOrgId, reset]);

  const platform = watch("platform");
  const targetOrgId = watch("target_org_id") ?? null;

  function onSubmit(data: CreateChannelBotFormData) {
    // Empty strings from the form should not be sent as target_org_id.
    const payload = {
      ...data,
      target_org_id:
        data.target_org_id && data.target_org_id.length > 0
          ? data.target_org_id
          : undefined,
    };
    createBot.mutate(payload, {
      onSuccess: (result) => {
        toast.success(
          `Bot "${result.platform_bot_username}" created successfully`,
        );
        reset();
        onOpenChange(false);
      },
      onError: (err) => {
        const message =
          err instanceof ApiError ? err.message : "Failed to create bot";
        toast.error(message);
      },
    });
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-md">
        <DialogHeader>
          <DialogTitle>Add Channel Bot</DialogTitle>
          <DialogDescription>
            Connect a messaging platform bot to relay messages to your AI agents.
          </DialogDescription>
        </DialogHeader>

        <form onSubmit={handleSubmit(onSubmit)} className="space-y-4">
          <div className="space-y-2">
            <Label htmlFor="scope">Scope</Label>
            <OrgScopeSelect
              value={targetOrgId}
              onChange={(next) =>
                setValue("target_org_id", next ?? undefined, {
                  shouldDirty: true,
                })
              }
              label="Scope"
            />
            <p className="text-xs text-muted-foreground">
              Choose where this bot lives. Org bots are visible to every
              org admin and can be bound to org-owned agent keys.
            </p>
          </div>

          <div className="space-y-2">
            <Label htmlFor="platform">Platform</Label>
            <Select
              value={platform}
              onValueChange={(value) =>
                setValue("platform", value as ChannelPlatform)
              }
            >
              <SelectTrigger>
                <SelectValue placeholder="Select platform" />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="telegram">Telegram</SelectItem>
                <SelectItem value="discord">Discord</SelectItem>
                <SelectItem value="lark">Lark</SelectItem>
                <SelectItem value="feishu">Feishu</SelectItem>
                <SelectItem value="slack">Slack</SelectItem>
              </SelectContent>
            </Select>
            {errors.platform && (
              <p className="text-xs text-destructive">
                {errors.platform.message}
              </p>
            )}
          </div>

          <div className="space-y-2">
            <Label htmlFor="label">Label</Label>
            <Input
              id="label"
              placeholder="My Telegram Bot"
              {...register("label")}
            />
            {errors.label && (
              <p className="text-xs text-destructive">
                {errors.label.message}
              </p>
            )}
          </div>

          <div className="space-y-2">
            <Label htmlFor="bot_token">Bot Token</Label>
            <Input
              id="bot_token"
              type="password"
              placeholder="Enter your bot token"
              {...register("bot_token")}
            />
            {errors.bot_token && (
              <p className="text-xs text-destructive">
                {errors.bot_token.message}
              </p>
            )}
          </div>

          {(platform === "lark" || platform === "feishu") && (
            <>
              <div className="space-y-2">
                <Label htmlFor="app_id">App ID</Label>
                <Input
                  id="app_id"
                  placeholder="cli_xxxxxxxxxx"
                  {...register("app_id")}
                />
                {errors.app_id && (
                  <p className="text-xs text-destructive">
                    {errors.app_id.message}
                  </p>
                )}
              </div>
              <div className="space-y-2">
                <Label htmlFor="app_secret">App Secret</Label>
                <Input
                  id="app_secret"
                  type="password"
                  placeholder="Enter app secret"
                  {...register("app_secret")}
                />
                {errors.app_secret && (
                  <p className="text-xs text-destructive">
                    {errors.app_secret.message}
                  </p>
                )}
              </div>
            </>
          )}

          {platform === "discord" && (
            <div className="space-y-2">
              <Label htmlFor="public_key">Public Key</Label>
              <Input
                id="public_key"
                placeholder="Discord application public key"
                {...register("public_key")}
              />
              {errors.public_key && (
                <p className="text-xs text-destructive">
                  {errors.public_key.message}
                </p>
              )}
            </div>
          )}

          {platform === "slack" && (
            <div className="space-y-2">
              <Label htmlFor="app_secret">Signing Secret</Label>
              <Input
                id="app_secret"
                type="password"
                placeholder="Slack app signing secret"
                {...register("app_secret")}
              />
              <p className="text-xs text-muted-foreground">
                Found under Basic Information → App Credentials in your
                Slack app settings. Used to verify Events API request
                signatures.
              </p>
              {errors.app_secret && (
                <p className="text-xs text-destructive">
                  {errors.app_secret.message}
                </p>
              )}
            </div>
          )}

          <DialogFooter>
            <Button
              type="button"
              variant="outline"
              onClick={() => onOpenChange(false)}
            >
              Cancel
            </Button>
            <Button type="submit" disabled={createBot.isPending}>
              {createBot.isPending ? "Creating..." : "Add Bot"}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}

function DeleteBotDialog({
  botId,
  onClose,
}: {
  readonly botId: string | null;
  readonly onClose: () => void;
}) {
  const deleteMutation = useDeleteChannelBot();

  async function handleDelete() {
    if (!botId) return;
    try {
      await deleteMutation.mutateAsync(botId);
      toast.success("Bot deleted");
    } catch (err) {
      toast.error(
        err instanceof ApiError ? err.message : "Failed to delete bot",
      );
    } finally {
      onClose();
    }
  }

  return (
    <Dialog open={botId !== null} onOpenChange={() => onClose()}>
      <DialogContent className="sm:max-w-md">
        <DialogHeader>
          <DialogTitle>Delete Channel Bot</DialogTitle>
          <DialogDescription>
            This will permanently delete this bot and all its conversation
            routes. This action cannot be undone.
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
            {deleteMutation.isPending ? "Deleting..." : "Delete"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

function LoadingSkeleton() {
  return (
    <div className="space-y-3">
      {Array.from({ length: 3 }, (_, i) => (
        <Skeleton key={i} className="h-14 w-full rounded-xl" />
      ))}
    </div>
  );
}

// ─────────────────────────────────────────────────────────────────────────────
// Device channels (HTTP Event Gateway, NyxID#221)
// ─────────────────────────────────────────────────────────────────────────────

function CreateDeviceChannelDialog({
  open,
  onOpenChange,
  defaultOrgId,
}: {
  readonly open: boolean;
  readonly onOpenChange: (open: boolean) => void;
  readonly defaultOrgId: string | null;
}) {
  const createDevice = useCreateDeviceConversation();
  const { data: apiKeys } = useApiKeys({ orgId: defaultOrgId });

  // Device events arrive through an agent callback URL, so only keys with
  // one configured are selectable.
  const activeApiKeys = useMemo(
    () => (apiKeys ?? []).filter((k) => k.is_active),
    [apiKeys],
  );

  const {
    register,
    handleSubmit,
    setValue,
    reset,
    formState: { errors },
  } = useForm<CreateDeviceConversationFormData>({
    resolver: zodResolver(createDeviceConversationSchema),
    defaultValues: {
      platform_conversation_id: "",
      agent_api_key_id: "",
      target_org_id: defaultOrgId ?? undefined,
    },
  });

  useEffect(() => {
    if (!open) return;
    reset({
      platform_conversation_id: "",
      agent_api_key_id: "",
      target_org_id: defaultOrgId ?? undefined,
    });
  }, [open, defaultOrgId, reset]);

  function onSubmit(data: CreateDeviceConversationFormData) {
    // react-hook-form's `register` submits `""` for blank text inputs.
    // Drop empty strings so the backend's default ("device") applies
    // instead of being overwritten with an empty conversation type.
    const conversationType =
      data.platform_conversation_type && data.platform_conversation_type.trim().length > 0
        ? data.platform_conversation_type.trim()
        : undefined;

    createDevice.mutate(
      {
        platform_conversation_id: data.platform_conversation_id,
        agent_api_key_id: data.agent_api_key_id,
        platform_conversation_type: conversationType,
        target_org_id:
          data.target_org_id && data.target_org_id.length > 0
            ? data.target_org_id
            : undefined,
      },
      {
        onSuccess: (created) => {
          // Surface the NyxID row _id — this is the path parameter for
          // POST /api/v1/channel-events/{conversation_id}, NOT the
          // logical `platform_conversation_id` the user typed in.
          toast.success(
            `Device channel created (ID: ${created.id}). Copy this ID — it's the path parameter for /channel-events.`,
          );
          reset();
          onOpenChange(false);
        },
        onError: (err) => {
          toast.error(
            err instanceof ApiError
              ? err.message
              : "Failed to create device channel",
          );
        },
      },
    );
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-md">
        <DialogHeader>
          <DialogTitle>Add Device Channel</DialogTitle>
          <DialogDescription>
            Device channels receive events through{" "}
            <code className="text-xs">
              POST /api/v1/channel-events/&#123;conversation_id&#125;
            </code>
            . No bot token is required — authenticate with the agent API key
            configured below.
          </DialogDescription>
        </DialogHeader>

        <form onSubmit={handleSubmit(onSubmit)} className="space-y-4">
          <div className="space-y-2">
            <Label htmlFor="platform_conversation_id">Device Channel ID</Label>
            <Input
              id="platform_conversation_id"
              placeholder="household-camera"
              {...register("platform_conversation_id")}
            />
            <p className="text-xs text-muted-foreground">
              The logical name the device will POST to. Must be unique per
              owner.
            </p>
            {errors.platform_conversation_id && (
              <p className="text-xs text-destructive">
                {errors.platform_conversation_id.message}
              </p>
            )}
          </div>

          <div className="space-y-2">
            <Label>Agent API Key</Label>
            {activeApiKeys.length === 0 ? (
              <p className="text-xs text-muted-foreground">
                No active agent API keys in this scope. Create one with a
                callback URL first.
              </p>
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
            <Label htmlFor="platform_conversation_type">
              Channel Type (optional)
            </Label>
            <Input
              id="platform_conversation_type"
              placeholder="device"
              {...register("platform_conversation_type")}
            />
            <p className="text-xs text-muted-foreground">
              Freeform label surfaced to the agent as
              <code className="mx-1 text-xs">conversation.conversation_type</code>
              (e.g. <code className="text-xs">camera</code>,{" "}
              <code className="text-xs">sensor</code>). Defaults to
              <code className="mx-1 text-xs">device</code>.
            </p>
          </div>

          <DialogFooter>
            <Button
              type="button"
              variant="outline"
              onClick={() => onOpenChange(false)}
            >
              Cancel
            </Button>
            <Button type="submit" disabled={createDevice.isPending}>
              {createDevice.isPending ? "Creating..." : "Create Device Channel"}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}

function DeviceChannelRow({
  conversation,
  onDelete,
}: {
  readonly conversation: ChannelConversationItem;
  readonly onDelete: (id: string) => void;
}) {
  async function copyRowId() {
    try {
      await navigator.clipboard.writeText(conversation.id);
      toast.success("Channel ID copied");
    } catch {
      toast.error("Could not copy — your browser blocked clipboard access");
    }
  }

  return (
    <TableRow>
      <TableCell>
        <div className="flex items-center gap-2">
          <code
            className="font-mono text-xs text-muted-foreground"
            title={conversation.id}
          >
            {conversation.id.slice(0, 8)}…
          </code>
          <Button
            variant="ghost"
            size="icon"
            className="h-6 w-6"
            onClick={() => void copyRowId()}
            title="Copy full channel ID (path parameter for /channel-events/{id})"
          >
            <Copy className="h-3 w-3 text-muted-foreground" />
          </Button>
        </div>
      </TableCell>
      <TableCell className="font-mono text-xs">
        {conversation.platform_conversation_id}
      </TableCell>
      <TableCell>
        <Badge variant="outline">{conversation.platform_conversation_type}</Badge>
      </TableCell>
      <TableCell className="font-mono text-xs text-muted-foreground">
        {conversation.agent_api_key_id.slice(0, 8)}…
      </TableCell>
      <TableCell>
        <Badge variant={conversation.is_active ? "success" : "secondary"}>
          {conversation.is_active ? "active" : "inactive"}
        </Badge>
      </TableCell>
      <TableCell className="text-xs text-muted-foreground">
        {formatDate(conversation.created_at)}
      </TableCell>
      <TableCell className="w-[60px]">
        <Button
          variant="ghost"
          size="icon"
          className="h-8 w-8"
          onClick={() => onDelete(conversation.id)}
        >
          <Trash2 className="h-4 w-4 text-muted-foreground" />
        </Button>
      </TableCell>
    </TableRow>
  );
}

function DeviceChannelsSection({
  orgId,
  onAdd,
}: {
  readonly orgId: string | null;
  readonly onAdd: () => void;
}) {
  const { data: conversations, isLoading } = useChannelConversations({ orgId });
  const deleteConversation = useDeleteChannelConversation();

  // Device channels live in the same `channel_conversations` collection as
  // bot routes. Filter client-side on `platform === "device"` rather than
  // add a new server-side filter.
  const devices = useMemo(
    () => (conversations ?? []).filter((c) => c.platform === "device"),
    [conversations],
  );

  function handleDelete(id: string) {
    deleteConversation.mutate(id, {
      onSuccess: () => toast.success("Device channel deleted"),
      onError: (err) =>
        toast.error(
          err instanceof ApiError ? err.message : "Failed to delete",
        ),
    });
  }

  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between">
        <div>
          <h2 className="text-lg font-semibold">Device Channels</h2>
          <p className="text-sm text-muted-foreground">
            HTTP Event Gateway channels for analyzers, sensors, and other
            non-bot event sources.
          </p>
        </div>
        <Button size="sm" variant="outline" onClick={onAdd}>
          <Plus className="mr-2 h-4 w-4" />
          Add Device Channel
        </Button>
      </div>

      {isLoading ? (
        <LoadingSkeleton />
      ) : devices.length === 0 ? (
        <Card>
          <CardContent className="flex flex-col items-center gap-3 py-10">
            <div className="flex h-12 w-12 items-center justify-center rounded-full border border-border">
              <Cpu className="h-5 w-5 text-muted-foreground" />
            </div>
            <p className="text-sm font-medium">No device channels yet</p>
            <p className="text-xs text-muted-foreground">
              Create one to let devices push events into the channel relay
              pipeline.
            </p>
          </CardContent>
        </Card>
      ) : (
        <div className="rounded-xl border border-border">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead title="NyxID conversation _id — path parameter for POST /api/v1/channel-events/{id}">
                  ID
                </TableHead>
                <TableHead>Channel Name</TableHead>
                <TableHead>Type</TableHead>
                <TableHead>Agent Key</TableHead>
                <TableHead>Status</TableHead>
                <TableHead>Created</TableHead>
                <TableHead className="w-[60px]" />
              </TableRow>
            </TableHeader>
            <TableBody>
              {devices.map((conversation) => (
                <DeviceChannelRow
                  key={conversation.id}
                  conversation={conversation}
                  onDelete={handleDelete}
                />
              ))}
            </TableBody>
          </Table>
        </div>
      )}
    </div>
  );
}

export function ChannelBotsPage() {
  const [scopeOrgId, setScopeOrgId] = useState<string | null>(null);
  const { data: bots, isLoading, error } = useChannelBots({ orgId: scopeOrgId });
  const [createOpen, setCreateOpen] = useState(false);
  const [createDeviceOpen, setCreateDeviceOpen] = useState(false);
  const [deleteTarget, setDeleteTarget] = useState<string | null>(null);

  return (
    <div className="space-y-8">
      <PageHeader
        title="Channel Bots"
        description="Manage messaging platform bots for agent relay."
        actions={
          <Button size="sm" onClick={() => setCreateOpen(true)}>
            <Plus className="mr-2 h-4 w-4" />
            Add Bot
          </Button>
        }
      />

      <div className="flex items-center gap-3">
        <Label htmlFor="bots-scope" className="text-sm font-medium">
          Scope
        </Label>
        <div className="w-60">
          <OrgScopeSelect value={scopeOrgId} onChange={setScopeOrgId} />
        </div>
      </div>

      {isLoading ? (
        <LoadingSkeleton />
      ) : error ? (
        <Card>
          <CardContent className="py-8 text-center text-sm text-destructive">
            Failed to load channel bots. Please try again.
          </CardContent>
        </Card>
      ) : !bots || bots.length === 0 ? (
        <EmptyState onAdd={() => setCreateOpen(true)} />
      ) : (
        <BotsTable bots={bots} onDelete={setDeleteTarget} />
      )}

      <DeviceChannelsSection
        orgId={scopeOrgId}
        onAdd={() => setCreateDeviceOpen(true)}
      />

      <CreateBotDialog
        open={createOpen}
        onOpenChange={setCreateOpen}
        defaultOrgId={scopeOrgId}
      />
      <CreateDeviceChannelDialog
        open={createDeviceOpen}
        onOpenChange={setCreateDeviceOpen}
        defaultOrgId={scopeOrgId}
      />
      <DeleteBotDialog botId={deleteTarget} onClose={() => setDeleteTarget(null)} />
    </div>
  );
}
