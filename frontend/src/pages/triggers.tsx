import { useUiTranslation } from "@/i18n/use-ui-translation";
import { translateUi } from "@/i18n/use-ui-translation";
import { translateErrorMessage } from "@/lib/error-messages";
import { Link } from "@tanstack/react-router";
import { useState } from "react";
import { TEXT_STYLES } from "@/lib/typography";
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
  if (trigger.delivery.type === "assistant") return translateUi("NyxBot / specialist");
  return trigger.delivery.type === "agent"
    ? translateUi("Agent")
    : trigger.delivery.type === "webhook"
      ? "Webhook"
      : translateUi("Notification");
}

function verificationLabel(trigger: TriggerResponse): string {
  if (trigger.verification.mode === "schedule") return translateUi("Scheduled");
  if (trigger.verification.mode === "hmac_sha256") return "HMAC-SHA256";
  return trigger.verification.location === "bearer"
    ? translateUi("Bearer token")
    : translateUi("Query token");
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
  useUiTranslation();
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button
          type="button"
          variant="ghost"
          size="icon"
          className="h-7 w-7"
          aria-label={translateUi("More actions for {{value1}}", { value1: trigger.label })}
        >
          <MoreVertical className="h-3.5 w-3.5" />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end">
        <DropdownMenuItem onClick={onView}>
          <Eye />{translateUi("View details")}</DropdownMenuItem>
        <DropdownMenuItem onClick={onRotate}>
          <RotateCw />{translateUi("Rotate secret")}</DropdownMenuItem>
        <DropdownMenuItem onClick={onToggle}>
          <Power />
          {trigger.status === "active" ? translateUi("Disable") : translateUi("Enable")}
        </DropdownMenuItem>
        <DropdownMenuItem
          className="text-destructive focus:text-destructive"
          onClick={onDelete}
        >
          <Trash2 />{translateUi("Delete")}</DropdownMenuItem>
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
  useUiTranslation();
  return (
    <Sheet open={open} onOpenChange={onOpenChange}>
      <SheetContent className="w-full overflow-y-auto sm:max-w-lg">
        <SheetHeader>
          <SheetTitle className="text-15">{trigger?.label}</SheetTitle>
          <SheetDescription>{translateUi("Ingress verification and delivery configuration.")}</SheetDescription>
        </SheetHeader>
        {trigger && (
          <div className="mt-6 space-y-4">
            <DetailSection title={translateUi("Trigger")}>
              <DetailRow
                label={translateUi("Status")}
                value={titleCase(trigger.status)}
                badge
              />
              <DetailRow
                label={translateUi("Delivery")}
                value={deliveryLabel(trigger)}
                badge
              />
              <DetailRow
                label={translateUi("Verification")}
                value={verificationLabel(trigger)}
              />
              <DetailRow
                label={translateUi("Inbound URL")}
                value={trigger.inbound_url ?? "No inbound URL"}
                copyable
                mono
              />
            </DetailSection>
            <DetailSection title={translateUi("Metadata")}>
              <DetailRow label={translateUi("Trigger ID")} value={trigger.id} copyable mono />
              <DetailRow
                label={translateUi("Created")}
                value={formatDateTime(trigger.created_at)}
              />
              <DetailRow
                label={translateUi("Updated")}
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
  useUiTranslation();
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
        { label: "Inbound URL", value: created.trigger.inbound_url ?? "" },
        { label: "Inbound Secret", value: created.secret ?? "" },
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
      toast.success(translateUi("Trigger created"));
    } catch (error) {
      toast.error(
        translateErrorMessage(error instanceof ApiError ? error.message : "Failed to create trigger"),
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
          <DialogTitle>{translateUi("Create Trigger")}</DialogTitle>
          <DialogDescription>{translateUi("Verify an inbound event and choose where NyxID should deliver it.")}</DialogDescription>
        </DialogHeader>
        <Form {...form}>
          <form className="space-y-4" onSubmit={form.handleSubmit(create)}>
            <FormField
              control={form.control}
              name="label"
              render={({ field }) => (
                <FormItem>
                  <FormLabel>{translateUi("Label")}</FormLabel>
                  <FormControl>
                    <Input {...field} placeholder={translateUi("Repository activity")} />
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
                  <FormLabel>{translateUi("Verification")}</FormLabel>
                  <Select value={field.value} onValueChange={field.onChange}>
                    <FormControl>
                      <SelectTrigger>
                        <SelectValue />
                      </SelectTrigger>
                    </FormControl>
                    <SelectContent>
                      <SelectItem value="bearer">{translateUi("Bearer token")}</SelectItem>
                      <SelectItem value="query">{translateUi("Query token")}</SelectItem>
                      <SelectItem value="hmac">{translateUi("HMAC-SHA256")}</SelectItem>
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
                    <FormLabel>{translateUi("Signature Header")}</FormLabel>
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
                  <FormLabel>{translateUi("Delivery Target")}</FormLabel>
                  <Select value={field.value} onValueChange={field.onChange}>
                    <FormControl>
                      <SelectTrigger>
                        <SelectValue />
                      </SelectTrigger>
                    </FormControl>
                    <SelectContent>
                      <SelectItem value="notification">{translateUi("Notification")}</SelectItem>
                      <SelectItem value="agent">{translateUi("Agent conversation")}</SelectItem>
                      <SelectItem value="webhook">{translateUi("Webhook")}</SelectItem>
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
                    <FormLabel>{translateUi("Webhook URL")}</FormLabel>
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
                    <FormLabel>{translateUi("Conversation ID")}</FormLabel>
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
              >{translateUi("Cancel")}</Button>
              <Button
                type="submit"
                variant="primary"
                disabled={
                  !form.formState.isDirty ||
                  !form.formState.isValid ||
                  createMutation.isPending
                }
                isLoading={createMutation.isPending}
              >{translateUi("Create")}</Button>
            </DialogFooter>
          </form>
        </Form>
      </DialogContent>
    </Dialog>
  );
}

export function TriggersPage() {
  useUiTranslation();
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
  const triggers =
    triggersQuery.data?.triggers.filter(
      (trigger) => trigger.source !== "schedule",
    ) ?? [];

  function reveal(title: string, values: readonly OneTimeSecretValue[]) {
    setSecretTitle(title);
    setSecretValues(values);
    setSecretOpen(true);
  }

  async function toggleStatus(trigger: TriggerResponse) {
    const status = trigger.status === "active" ? "disabled" : "active";
    try {
      await updateMutation.mutateAsync({ id: trigger.id, data: { status } });
      toast.success(status === "active" ? translateUi("Trigger enabled") : translateUi("Trigger disabled"));
    } catch (error) {
      toast.error(
        translateErrorMessage(error instanceof ApiError ? error.message : "Failed to update trigger"),
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
      toast.success(translateUi("Trigger secret rotated"));
    } catch (error) {
      toast.error(
        translateErrorMessage(error instanceof ApiError
          ? error.message
          : "Failed to rotate trigger secret"),
      );
    }
  }

  async function deleteTrigger() {
    if (!deleteTarget) return;
    try {
      await deleteMutation.mutateAsync(deleteTarget.id);
      setDeleteTarget(null);
      toast.success(translateUi("Trigger deleted"));
    } catch (error) {
      toast.error(
        translateErrorMessage(error instanceof ApiError ? error.message : "Failed to delete trigger"),
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
        title={translateUi("Triggers")}
        description={translateUi("Relay verified inbound events to agents, webhooks, or notifications.")}
        actions={
          <>
            <Link to="/assistant/automations" className="text-14 text-primary-text underline">{translateUi("Automations")}</Link>
            <AddCtaButton
              label={translateUi("Create Trigger")}
              onClick={() => setCreateOpen(true)}
            />
          </>
        }
      />

      {triggersQuery.isLoading ? (
        <div className="space-y-3">
          <Skeleton className="h-20 w-full" />
          <Skeleton className="h-20 w-full" />
        </div>
      ) : triggersQuery.error ? (
        <ErrorBanner
          message={translateUi("Failed to load triggers.")}
          onRetry={() => void triggersQuery.refetch()}
        />
      ) : triggers.length === 0 ? (
        <TeachingEmptyState
          icon={DishAntennaIcon}
          title={translateUi("No triggers yet")}
          description={translateUi("Create an authenticated ingress endpoint and choose where events should go.")}
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
                  <div className="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg border border-border/50 bg-overlay">
                    <BellRing className="h-4 w-4 text-muted-foreground" />
                  </div>
                  <div className="min-w-0 space-y-2">
                    <p data-trigger-title="" className={`truncate ${TEXT_STYLES.rowTitle}`}>
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
                    <p className="text-13 text-text-tertiary">{translateUi("Created ")}{formatDateTime(trigger.created_at)}
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
                  <TableHead>{translateUi("Label")}</TableHead>
                  <TableHead>{translateUi("Delivery")}</TableHead>
                  <TableHead>{translateUi("Status")}</TableHead>
                  <TableHead>{translateUi("Created")}</TableHead>
                  <TableHead className="w-12 text-right">{translateUi("Actions")}</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {triggers.map((trigger) => (
                  <TableRow key={trigger.id}>
                    <TableCell>
                      <span data-trigger-title="" className={TEXT_STYLES.rowTitle}>{trigger.label}</span>
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
                    <TableCell className="font-mono text-14 text-text-tertiary">
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
        description={translateUi("These values are shown only once. Copy and store them securely now.")}
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
            <DialogTitle>{translateUi("Rotate trigger secret?")}</DialogTitle>
            <DialogDescription>{translateUi("The current inbound secret will stop working immediately.")}</DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button variant="outline" onClick={() => setRotateTarget(null)}>{translateUi("Cancel")}</Button>
            <Button
              variant="primary"
              isLoading={rotateMutation.isPending}
              onClick={() => void rotateSecret()}
            >{translateUi("Rotate Secret")}</Button>
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
            <DialogTitle>{translateUi("Delete trigger?")}</DialogTitle>
            <DialogDescription>{translateUi("The inbound URL and secret will stop accepting events. This action cannot be undone.")}</DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button variant="outline" onClick={() => setDeleteTarget(null)}>{translateUi("Cancel")}</Button>
            <Button
              variant="destructive"
              isLoading={deleteMutation.isPending}
              onClick={() => void deleteTrigger()}
            >{translateUi("Delete Trigger")}</Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}
