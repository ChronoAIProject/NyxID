import { useState } from "react";
import { useNavigate } from "@tanstack/react-router";
import { useForm } from "react-hook-form";
import { zodResolver } from "@hookform/resolvers/zod";
import { useChannelBots, useCreateChannelBot, useDeleteChannelBot } from "@/hooks/use-channel-bots";
import { createChannelBotSchema, type CreateChannelBotFormData } from "@/schemas/channels";
import { ApiError } from "@/lib/api-client";
import { formatDate } from "@/lib/utils";
import { PageHeader } from "@/components/shared/page-header";
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
import { Bot, Check, Plus, Trash2 } from "lucide-react";
import { toast } from "sonner";
import type { ChannelBotItem, ChannelBotStatus, ChannelPlatform } from "@/types/channels";

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
}: {
  readonly open: boolean;
  readonly onOpenChange: (open: boolean) => void;
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
    },
  });

  const platform = watch("platform");

  function onSubmit(data: CreateChannelBotFormData) {
    createBot.mutate(data, {
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

export function ChannelBotsPage() {
  const { data: bots, isLoading, error } = useChannelBots();
  const [createOpen, setCreateOpen] = useState(false);
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

      <CreateBotDialog open={createOpen} onOpenChange={setCreateOpen} />
      <DeleteBotDialog botId={deleteTarget} onClose={() => setDeleteTarget(null)} />
    </div>
  );
}
