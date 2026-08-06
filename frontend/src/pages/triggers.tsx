import { useState } from "react";
import { zodResolver } from "@hookform/resolvers/zod";
import {
  BellRing,
  Eye,
  MoreVertical,
  Power,
  RotateCw,
  Trash2,
  Webhook,
} from "lucide-react";
import { toast } from "sonner";
import {
  useCreateTrigger,
  useDeleteTrigger,
  useRotateTriggerSecret,
  useTriggers,
  useUpdateTrigger,
} from "@/hooks/use-triggers";
import {
  buildCreateTriggerRequest,
  triggerFormSchema,
  type TriggerForm,
  type TriggerResponse,
} from "@/schemas/triggers";
import { ApiError } from "@/lib/api-client";
import { formatDateTime } from "@/lib/utils";
import { AddCtaButton } from "@/components/shared/add-cta-button";
import { DetailRow } from "@/components/shared/detail-row";
import { DetailSection } from "@/components/shared/detail-section";
import { ErrorBanner } from "@/components/shared/error-banner";
import {
  OneTimeSecretDialog,
  type OneTimeSecretValue,
} from "@/components/shared/one-time-secret-dialog";
import { PageHeader } from "@/components/shared/page-header";
import { TeachingEmptyState } from "@/components/shared/teaching-empty-state";
import { DishAntennaIcon } from "@/components/icons/empty-state";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import {
  Form,
  FormControl,
  FormField,
  FormItem,
  FormLabel,
  FormMessage,
  useAppForm,
} from "@/components/ui/form";
import { Input } from "@/components/ui/input";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import {
  Sheet,
  SheetContent,
  SheetDescription,
  SheetHeader,
  SheetTitle,
} from "@/components/ui/sheet";
import { Skeleton } from "@/components/ui/skeleton";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";

const TRIGGER_DEFAULT_VALUES: TriggerForm = {
  label: "",
  verification_mode: "bearer",
  signature_header: "X-Hub-Signature-256",
  delivery_type: "notification",
  webhook_url: "",
  conversation_id: "",
};

function titleCase(value: string): string {
  return value.charAt(0).toUpperCase() + value.slice(1).replaceAll("_", " ");
}

function deliveryLabel(trigger: TriggerResponse): string {
  return trigger.delivery.type === "agent"
    ? "Agent"
    : trigger.delivery.type === "webhook"
      ? "Webhook"
      : "Notification";
}

function verificationLabel(trigger: TriggerResponse): string {
  if (trigger.verification.mode === "hmac_sha256") return "HMAC-SHA256";
  return trigger.verification.location === "bearer"
    ? "Bearer token"
    : "Query token";
}

function TriggerActions({
  trigger,
  onView,
  onRotate,
  onToggle,
  onDelete,
}: {
  readonly trigger: TriggerResponse;
  readonly onView: () => void;
  readonly onRotate: () => void;
  readonly onToggle: () => void;
  readonly onDelete: () => void;
}) {
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button
          type="button"
          variant="ghost"
          size="icon"
          className="h-7 w-7"
          aria-label={`More actions for ${trigger.label}`}
        >
          <MoreVertical className="h-3.5 w-3.5" />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end">
        <DropdownMenuItem onClick={onView}>
          <Eye />
          View details
        </DropdownMenuItem>
        <DropdownMenuItem onClick={onRotate}>
          <RotateCw />
          Rotate secret
        </DropdownMenuItem>
        <DropdownMenuItem onClick={onToggle}>
          <Power />
          {trigger.status === "active" ? "Disable" : "Enable"}
        </DropdownMenuItem>
        <DropdownMenuItem
          className="text-destructive focus:text-destructive"
          onClick={onDelete}
        >
          <Trash2 />
          Delete
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

function TriggerDetail({
  trigger,
  open,
  onOpenChange,
}: {
  readonly trigger: TriggerResponse | null;
  readonly open: boolean;
  readonly onOpenChange: (open: boolean) => void;
}) {
  return (
    <Sheet open={open} onOpenChange={onOpenChange}>
      <SheetContent className="w-full overflow-y-auto sm:max-w-lg">
        <SheetHeader>
          <SheetTitle className="text-[15px]">{trigger?.label}</SheetTitle>
          <SheetDescription>
            Ingress verification and delivery configuration.
          </SheetDescription>
        </SheetHeader>
        {trigger && (
          <div className="mt-6 space-y-4">
            <DetailSection title="Trigger">
              <DetailRow
                label="Status"
                value={titleCase(trigger.status)}
                badge
              />
              <DetailRow
                label="Delivery"
                value={deliveryLabel(trigger)}
                badge
              />
              <DetailRow
                label="Verification"
                value={verificationLabel(trigger)}
              />
              <DetailRow
                label="Inbound URL"
                value={trigger.inbound_url}
                copyable
                mono
              />
            </DetailSection>
            <DetailSection title="Metadata">
              <DetailRow label="Trigger ID" value={trigger.id} copyable mono />
              <DetailRow
                label="Created"
                value={formatDateTime(trigger.created_at)}
              />
              <DetailRow
                label="Updated"
                value={formatDateTime(trigger.updated_at)}
              />
            </DetailSection>
          </div>
        )}
      </SheetContent>
    </Sheet>
  );
}

function TriggerCreateDialog({
  open,
  onOpenChange,
  onCreated,
}: {
  readonly open: boolean;
  readonly onOpenChange: (open: boolean) => void;
  readonly onCreated: (values: readonly OneTimeSecretValue[]) => void;
}) {
  const createMutation = useCreateTrigger();
  const form = useAppForm<TriggerForm>({
    resolver: zodResolver(triggerFormSchema),
    mode: "onChange",
    defaultValues: TRIGGER_DEFAULT_VALUES,
  });
  const verificationMode = form.watch("verification_mode");
  const deliveryType = form.watch("delivery_type");

  async function create(values: TriggerForm) {
    try {
      const created = await createMutation.mutateAsync(
        buildCreateTriggerRequest(values),
      );
      const oneTimeValues: OneTimeSecretValue[] = [
        { label: "Inbound URL", value: created.trigger.inbound_url },
        { label: "Inbound Secret", value: created.secret },
      ];
      if (created.delivery_signing_secret) {
        oneTimeValues.push({
          label: "Delivery Signing Secret",
          value: created.delivery_signing_secret,
        });
      }
      onOpenChange(false);
      form.reset(TRIGGER_DEFAULT_VALUES);
      onCreated(oneTimeValues);
      toast.success("Trigger created");
    } catch (error) {
      toast.error(
        error instanceof ApiError ? error.message : "Failed to create trigger",
      );
    }
  }

  function handleOpenChange(next: boolean) {
    onOpenChange(next);
    if (!next) form.reset(TRIGGER_DEFAULT_VALUES);
  }

  return (
    <Dialog open={open} onOpenChange={handleOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Create Trigger</DialogTitle>
          <DialogDescription>
            Verify an inbound event and choose where NyxID should deliver it.
          </DialogDescription>
        </DialogHeader>
        <Form {...form}>
          <form className="space-y-4" onSubmit={form.handleSubmit(create)}>
            <FormField
              control={form.control}
              name="label"
              render={({ field }) => (
                <FormItem>
                  <FormLabel>Label</FormLabel>
                  <FormControl>
                    <Input {...field} placeholder="Repository activity" />
                  </FormControl>
                  <FormMessage />
                </FormItem>
              )}
            />
            <FormField
              control={form.control}
              name="verification_mode"
              render={({ field }) => (
                <FormItem>
                  <FormLabel>Verification</FormLabel>
                  <Select value={field.value} onValueChange={field.onChange}>
                    <FormControl>
                      <SelectTrigger>
                        <SelectValue />
                      </SelectTrigger>
                    </FormControl>
                    <SelectContent>
                      <SelectItem value="bearer">Bearer token</SelectItem>
                      <SelectItem value="query">Query token</SelectItem>
                      <SelectItem value="hmac">HMAC-SHA256</SelectItem>
                    </SelectContent>
                  </Select>
                  <FormMessage />
                </FormItem>
              )}
            />
            {verificationMode === "hmac" && (
              <FormField
                control={form.control}
                name="signature_header"
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>Signature Header</FormLabel>
                    <FormControl>
                      <Input {...field} placeholder="X-Hub-Signature-256" />
                    </FormControl>
                    <FormMessage />
                  </FormItem>
                )}
              />
            )}
            <FormField
              control={form.control}
              name="delivery_type"
              render={({ field }) => (
                <FormItem>
                  <FormLabel>Delivery Target</FormLabel>
                  <Select value={field.value} onValueChange={field.onChange}>
                    <FormControl>
                      <SelectTrigger>
                        <SelectValue />
                      </SelectTrigger>
                    </FormControl>
                    <SelectContent>
                      <SelectItem value="notification">Notification</SelectItem>
                      <SelectItem value="agent">Agent conversation</SelectItem>
                      <SelectItem value="webhook">Webhook</SelectItem>
                    </SelectContent>
                  </Select>
                  <FormMessage />
                </FormItem>
              )}
            />
            {deliveryType === "webhook" && (
              <FormField
                control={form.control}
                name="webhook_url"
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>Webhook URL</FormLabel>
                    <FormControl>
                      <Input
                        {...field}
                        type="url"
                        placeholder="https://events.example.com/inbound"
                      />
                    </FormControl>
                    <FormMessage />
                  </FormItem>
                )}
              />
            )}
            {deliveryType === "agent" && (
              <FormField
                control={form.control}
                name="conversation_id"
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>Conversation ID</FormLabel>
                    <FormControl>
                      <Input {...field} placeholder="conversation-id" />
                    </FormControl>
                    <FormMessage />
                  </FormItem>
                )}
              />
            )}
            <DialogFooter>
              <Button
                type="button"
                variant="outline"
                onClick={() => handleOpenChange(false)}
              >
                Cancel
              </Button>
              <Button
                type="submit"
                variant="primary"
                disabled={
                  !form.formState.isDirty ||
                  !form.formState.isValid ||
                  createMutation.isPending
                }
                isLoading={createMutation.isPending}
              >
                Create
              </Button>
            </DialogFooter>
          </form>
        </Form>
      </DialogContent>
    </Dialog>
  );
}

export function TriggersPage() {
  const triggersQuery = useTriggers();
  const updateMutation = useUpdateTrigger();
  const deleteMutation = useDeleteTrigger();
  const rotateMutation = useRotateTriggerSecret();
  const [createOpen, setCreateOpen] = useState(false);
  const [detailTarget, setDetailTarget] = useState<TriggerResponse | null>(
    null,
  );
  const [rotateTarget, setRotateTarget] = useState<TriggerResponse | null>(
    null,
  );
  const [deleteTarget, setDeleteTarget] = useState<TriggerResponse | null>(
    null,
  );
  const [secretOpen, setSecretOpen] = useState(false);
  const [secretTitle, setSecretTitle] = useState("Save Trigger Credentials");
  const [secretValues, setSecretValues] = useState<
    readonly OneTimeSecretValue[]
  >([]);
  const triggers = triggersQuery.data?.triggers ?? [];

  function reveal(title: string, values: readonly OneTimeSecretValue[]) {
    setSecretTitle(title);
    setSecretValues(values);
    setSecretOpen(true);
  }

  async function toggleStatus(trigger: TriggerResponse) {
    const status = trigger.status === "active" ? "disabled" : "active";
    try {
      await updateMutation.mutateAsync({ id: trigger.id, data: { status } });
      toast.success(`Trigger ${status === "active" ? "enabled" : "disabled"}`);
    } catch (error) {
      toast.error(
        error instanceof ApiError ? error.message : "Failed to update trigger",
      );
    }
  }

  async function rotateSecret() {
    if (!rotateTarget) return;
    try {
      const response = await rotateMutation.mutateAsync(rotateTarget.id);
      setRotateTarget(null);
      reveal("Save Rotated Trigger Secret", [
        { label: "Inbound Secret", value: response.secret },
      ]);
      toast.success("Trigger secret rotated");
    } catch (error) {
      toast.error(
        error instanceof ApiError
          ? error.message
          : "Failed to rotate trigger secret",
      );
    }
  }

  async function deleteTrigger() {
    if (!deleteTarget) return;
    try {
      await deleteMutation.mutateAsync(deleteTarget.id);
      setDeleteTarget(null);
      toast.success("Trigger deleted");
    } catch (error) {
      toast.error(
        error instanceof ApiError ? error.message : "Failed to delete trigger",
      );
    }
  }

  const actions = (trigger: TriggerResponse) => (
    <TriggerActions
      trigger={trigger}
      onView={() => setDetailTarget(trigger)}
      onRotate={() => setRotateTarget(trigger)}
      onToggle={() => void toggleStatus(trigger)}
      onDelete={() => setDeleteTarget(trigger)}
    />
  );

  return (
    <div className="space-y-8">
      <PageHeader
        title="Triggers"
        description="Relay verified inbound events to agents, webhooks, or notifications."
        actions={
          <AddCtaButton
            label="Create Trigger"
            onClick={() => setCreateOpen(true)}
          />
        }
      />

      {triggersQuery.isLoading ? (
        <div className="space-y-3">
          <Skeleton className="h-20 w-full" />
          <Skeleton className="h-20 w-full" />
        </div>
      ) : triggersQuery.error ? (
        <ErrorBanner
          message="Failed to load triggers."
          onRetry={() => void triggersQuery.refetch()}
        />
      ) : triggers.length === 0 ? (
        <TeachingEmptyState
          icon={DishAntennaIcon}
          title="No triggers yet"
          description="Create an authenticated ingress endpoint and choose where events should go."
          primaryCta={{
            label: "Create Your First Trigger",
            onClick: () => setCreateOpen(true),
            icon: Webhook,
          }}
        />
      ) : (
        <>
          <div className="flex flex-col gap-3 md:hidden">
            {triggers.map((trigger) => (
              <div
                key={trigger.id}
                className="relative rounded-xl border border-border/50 bg-card p-4 pr-12"
              >
                <div className="absolute right-3 top-3">{actions(trigger)}</div>
                <div className="flex items-start gap-3">
                  <div className="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg border border-border/50 bg-white/[0.03]">
                    <BellRing className="h-4 w-4 text-muted-foreground" />
                  </div>
                  <div className="min-w-0 space-y-2">
                    <p className="truncate text-[13px] font-semibold">
                      {trigger.label}
                    </p>
                    <div className="flex flex-wrap gap-1.5">
                      <Badge
                        variant={
                          trigger.status === "active" ? "success" : "secondary"
                        }
                      >
                        {titleCase(trigger.status)}
                      </Badge>
                      <Badge variant="secondary">
                        {deliveryLabel(trigger)}
                      </Badge>
                    </div>
                    <p className="text-[11px] text-text-tertiary">
                      Created {formatDateTime(trigger.created_at)}
                    </p>
                  </div>
                </div>
              </div>
            ))}
          </div>

          <div className="hidden overflow-hidden rounded-xl border border-border/50 bg-card md:block">
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>Label</TableHead>
                  <TableHead>Delivery</TableHead>
                  <TableHead>Status</TableHead>
                  <TableHead>Created</TableHead>
                  <TableHead className="w-12 text-right">Actions</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {triggers.map((trigger) => (
                  <TableRow key={trigger.id}>
                    <TableCell className="font-medium">
                      {trigger.label}
                    </TableCell>
                    <TableCell>{deliveryLabel(trigger)}</TableCell>
                    <TableCell>
                      <Badge
                        variant={
                          trigger.status === "active" ? "success" : "secondary"
                        }
                      >
                        {titleCase(trigger.status)}
                      </Badge>
                    </TableCell>
                    <TableCell className="font-mono text-[11px] text-text-tertiary">
                      {formatDateTime(trigger.created_at)}
                    </TableCell>
                    <TableCell className="text-right">
                      {actions(trigger)}
                    </TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          </div>
        </>
      )}

      <TriggerCreateDialog
        open={createOpen}
        onOpenChange={setCreateOpen}
        onCreated={(values) => reveal("Save Trigger Credentials", values)}
      />
      <TriggerDetail
        trigger={detailTarget}
        open={detailTarget !== null}
        onOpenChange={(open) => {
          if (!open) setDetailTarget(null);
        }}
      />
      <OneTimeSecretDialog
        open={secretOpen}
        onOpenChange={setSecretOpen}
        title={secretTitle}
        description="These values are shown only once. Copy and store them securely now."
        values={secretValues}
      />

      <Dialog
        open={rotateTarget !== null}
        onOpenChange={(open) => {
          if (!open) setRotateTarget(null);
        }}
      >
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Rotate trigger secret?</DialogTitle>
            <DialogDescription>
              The current inbound secret will stop working immediately.
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button variant="outline" onClick={() => setRotateTarget(null)}>
              Cancel
            </Button>
            <Button
              variant="primary"
              isLoading={rotateMutation.isPending}
              onClick={() => void rotateSecret()}
            >
              Rotate Secret
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <Dialog
        open={deleteTarget !== null}
        onOpenChange={(open) => {
          if (!open) setDeleteTarget(null);
        }}
      >
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Delete trigger?</DialogTitle>
            <DialogDescription>
              The inbound URL and secret will stop accepting events. This action
              cannot be undone.
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button variant="outline" onClick={() => setDeleteTarget(null)}>
              Cancel
            </Button>
            <Button
              variant="destructive"
              isLoading={deleteMutation.isPending}
              onClick={() => void deleteTrigger()}
            >
              Delete Trigger
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}
