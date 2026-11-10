import { translateUi, useUiTranslation } from "@/i18n/use-ui-translation";

import { useRef, useState } from "react";
import { zodResolver } from "@hookform/resolvers/zod";
import { toast } from "sonner";
import { useAppForm } from "@/components/ui/form";
import { OwnerSelect, type SelectedOwner } from "./owner-select";
import {
  useOwnershipTransfer,
  useOwnershipTransferPreview,
} from "@/hooks/use-ownership-transfers";
import {
  ownershipTransferSchema,
  type OwnershipTransferForm,
} from "@/schemas/ownership-transfers";
import type {
  OwnershipResource,
  OwnershipResourceKind,
  OwnershipTransferPreview,
} from "@/types/ownership-transfers";
import { ApiError } from "@/lib/api-client";
import { Button } from "@/components/ui/button";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";

function errorMessage(error: unknown): string {
  return error instanceof Error
    ? error.message
    : "Ownership transfer failed. Please retry.";
}

export function OwnershipTransferDialog({
  kind,
  resource,
  onClose,
  onTransferred,
}: {
  readonly kind: OwnershipResourceKind;
  readonly resource: OwnershipResource;
  readonly onClose: () => void;
  readonly onTransferred?: () => void;
}) {
  const { t } = useUiTranslation();
  const form = useAppForm<OwnershipTransferForm>({
    resolver: zodResolver(ownershipTransferSchema),
    defaultValues: { new_owner_user_id: "" },
  });
  const [ownerType, setOwnerType] = useState<"person" | "org">("org");
  const [selectedOwner, setSelectedOwner] = useState<SelectedOwner | null>(
    null,
  );
  const [review, setReview] = useState<{
    preview: OwnershipTransferPreview;
    requestId: string;
  } | null>(null);
  const [error, setError] = useState<string | null>(null);
  const busy = useRef(false);
  const previewMutation = useOwnershipTransferPreview(kind, resource.id);
  const transferMutation = useOwnershipTransfer(kind, resource.id);
  const pending = previewMutation.isPending || transferMutation.isPending;
  const selectedId = form.watch("new_owner_user_id");

  async function prepare(values: OwnershipTransferForm) {
    if (busy.current) return;
    busy.current = true;
    setError(null);
    try {
      const preview = await previewMutation.mutateAsync(
        values.new_owner_user_id,
      );
      setReview({ preview, requestId: crypto.randomUUID() });
    } catch (cause) {
      setError(errorMessage(cause));
    } finally {
      busy.current = false;
    }
  }

  async function confirm() {
    if (!review || busy.current || review.preview.blockers.length > 0) return;
    busy.current = true;
    setError(null);
    try {
      await transferMutation.mutateAsync({
        new_owner_user_id: review.preview.new_owner_user_id,
        expected_version: review.preview.version,
        request_id: review.requestId,
      });
      toast.success(translateUi("Ownership of {{name}} transferred", { name: review.preview.name }));
      onClose();
      onTransferred?.();
    } catch (cause) {
      setError(errorMessage(cause));
      if (cause instanceof ApiError && cause.status === 409) setReview(null);
    } finally {
      busy.current = false;
    }
  }

  return (
    <Dialog
      open
      onOpenChange={(open) => {
        if (!open && !busy.current) onClose();
      }}
    >
      <DialogContent>
        <DialogHeader>
          <DialogTitle>
            {review ? t("Review ownership transfer") : t("Transfer ownership")}
          </DialogTitle>
          <DialogDescription>{t("Move ")}{resource.name}{t(" to a specific person or organization.")}</DialogDescription>
        </DialogHeader>
        {review ? (
          <div className="space-y-4 text-sm">
            <dl className="space-y-2 rounded-md border p-3">
              <dt className="text-muted-foreground">{t("Current owner")}</dt>
              <dd className="break-all">
                {review.preview.previous_owner_name ??
                  review.preview.previous_owner_user_id}
                {review.preview.previous_owner_name && (
                  <span className="block text-xs text-muted-foreground">
                    {review.preview.previous_owner_user_id}
                  </span>
                )}
              </dd>
              <dt className="text-muted-foreground">{t("Destination")}{" "}
                {review.preview.destination_type === "org"
                  ? t("organization")
                  : t("person")}
              </dt>
              <dd>
                {review.preview.destination_name}
                <span className="block break-all text-xs text-muted-foreground">
                  {review.preview.new_owner_user_id}
                </span>
              </dd>
            </dl>
            <ul className="list-disc space-y-2 pl-5">
              {review.preview.effects.map((effect) => (
                <li key={effect}>{effect}</li>
              ))}
            </ul>
            {kind === "channel_bot" && (
              <p>
                {review.preview.routes_to_retire}{t(" routes will be permanently retired.")}</p>
            )}
            {review.preview.blockers.length > 0 && (
              <div
                role="alert"
                className="rounded-md border border-destructive p-3 text-destructive"
              >
                <p className="font-medium">{t("This resource cannot be transferred")}</p>
                <ul className="mt-2 list-disc space-y-2 pl-5">
                  {review.preview.blockers.map((blocker) => (
                    <li key={blocker}>{blocker}</li>
                  ))}
                </ul>
              </div>
            )}
          </div>
        ) : (
          <form
            id="ownership-transfer"
            onSubmit={form.handleSubmit(prepare)}
            className="space-y-4"
          >
            <div className="space-y-2">
              <Label htmlFor="destination-type">{t("Destination type")}</Label>
              <Select
                value={ownerType}
                disabled={pending}
                onValueChange={(value) => {
                  setOwnerType(value as "person" | "org");
                  form.setValue("new_owner_user_id", "");
                  setSelectedOwner(null);
                }}
              >
                <SelectTrigger id="destination-type">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="org">{t("Organization")}</SelectItem>
                  <SelectItem value="person">{t("Person")}</SelectItem>
                </SelectContent>
              </Select>
            </div>
            <div className="space-y-2">
              <Label htmlFor="destination-owner">{t("Destination owner")}</Label>
              <OwnerSelect
                kind={kind}
                resourceId={resource.id}
                key={ownerType}
                id="destination-owner"
                ownerType={ownerType}
                currentOwnerId={resource.owner_user_id}
                value={selectedOwner}
                disabled={pending}
                onChange={(owner) => {
                  form.setValue("new_owner_user_id", owner.id);
                  setSelectedOwner(owner);
                }}
              />
              {form.formState.errors.new_owner_user_id && (
                <p role="alert" className="text-sm text-destructive">
                  {translateUi(form.formState.errors.new_owner_user_id.message ?? "")}
                </p>
              )}
            </div>
          </form>
        )}
        {error && (
          <p role="alert" className="text-sm text-destructive">
            {translateUi(error)}
          </p>
        )}
        <DialogFooter>
          <Button
            type="button"
            variant="outline"
            disabled={pending}
            onClick={() => {
              if (review) {
                setReview(null);
                setError(null);
              } else onClose();
            }}
          >
            {review ? t("Back") : t("Cancel")}
          </Button>
          {review ? (
            <Button
              type="button"
              disabled={pending || review.preview.blockers.length > 0}
              onClick={() => void confirm()}
            >
              {pending ? t("Transferring…") : t("Confirm transfer")}
            </Button>
          ) : (
            <Button
              type="submit"
              form="ownership-transfer"
              disabled={pending || !selectedId}
            >
              {pending ? t("Preparing…") : t("Review transfer")}
            </Button>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
