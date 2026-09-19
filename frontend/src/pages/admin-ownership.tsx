import { useRef, useState } from "react";
import { zodResolver } from "@hookform/resolvers/zod";
import { toast } from "sonner";
import { useAppForm } from "@/components/ui/form";
import { useAuthStore } from "@/stores/auth-store";
import { canAdminWrite } from "@/types/api";
import { useAdminUsers } from "@/hooks/use-admin";
import {
  useOwnershipResources,
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
import { PageHeader } from "@/components/shared/page-header";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
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
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";

function errorMessage(error: unknown): string {
  return error instanceof Error
    ? error.message
    : "Ownership transfer failed. Please retry.";
}

export function OwnershipTransferDialog({
  kind,
  resource,
  onClose,
}: {
  readonly kind: OwnershipResourceKind;
  readonly resource: OwnershipResource;
  readonly onClose: () => void;
}) {
  const form = useAppForm<OwnershipTransferForm>({
    resolver: zodResolver(ownershipTransferSchema),
    defaultValues: { new_owner_user_id: "" },
  });
  const [ownerType, setOwnerType] = useState<"person" | "org">("org");
  const [search, setSearch] = useState("");
  const [ownerPage, setOwnerPage] = useState(1);
  const [selectedLabel, setSelectedLabel] = useState("");
  const [review, setReview] = useState<{
    preview: OwnershipTransferPreview;
    requestId: string;
  } | null>(null);
  const [error, setError] = useState<string | null>(null);
  const busy = useRef(false);
  const owners = useAdminUsers(ownerPage, 20, search || undefined, ownerType);
  const previewMutation = useOwnershipTransferPreview(kind, resource.id);
  const transferMutation = useOwnershipTransfer(kind, resource.id);
  const pending = previewMutation.isPending || transferMutation.isPending;
  const selectedId = form.watch("new_owner_user_id");
  const candidates = owners.data?.users ?? [];

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
      toast.success(`Ownership of ${review.preview.name} transferred`);
      onClose();
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
      <DialogContent className="max-h-[90vh] overflow-y-auto">
        <DialogHeader>
          <DialogTitle>
            {review ? "Review ownership transfer" : "Transfer ownership"}
          </DialogTitle>
          <DialogDescription>
            Move {resource.name} to a specific person or organization.
          </DialogDescription>
        </DialogHeader>
        {review ? (
          <div className="space-y-4 text-sm">
            <dl className="space-y-2 rounded-md border p-3">
              <dt className="text-muted-foreground">Current owner</dt>
              <dd className="break-all">
                {review.preview.previous_owner_name ??
                  review.preview.previous_owner_user_id}
                {review.preview.previous_owner_name && (
                  <span className="block text-xs text-muted-foreground">
                    {review.preview.previous_owner_user_id}
                  </span>
                )}
              </dd>
              <dt className="text-muted-foreground">
                Destination{" "}
                {review.preview.destination_type === "org"
                  ? "organization"
                  : "person"}
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
                {review.preview.routes_to_retire} routes will be permanently
                retired.
              </p>
            )}
            {review.preview.blockers.length > 0 && (
              <div
                role="alert"
                className="rounded-md border border-destructive p-3 text-destructive"
              >
                <p className="font-medium">
                  This resource cannot be transferred
                </p>
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
              <Label htmlFor="destination-type">Destination type</Label>
              <Select
                value={ownerType}
                disabled={pending}
                onValueChange={(value) => {
                  setOwnerType(value as "person" | "org");
                  setOwnerPage(1);
                  form.setValue("new_owner_user_id", "");
                  setSelectedLabel("");
                }}
              >
                <SelectTrigger id="destination-type">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="org">Organization</SelectItem>
                  <SelectItem value="person">Person</SelectItem>
                </SelectContent>
              </Select>
            </div>
            <div className="space-y-2">
              <Label htmlFor="owner-search">Find destination</Label>
              <Input
                id="owner-search"
                value={search}
                disabled={pending}
                placeholder="Search name or email"
                onChange={(event) => {
                  setSearch(event.target.value);
                  setOwnerPage(1);
                }}
              />
            </div>
            <div className="space-y-2">
              <Label htmlFor="destination-owner">Destination owner</Label>
              <Select
                value={selectedId}
                disabled={pending || owners.isLoading}
                onValueChange={(id) => {
                  form.setValue("new_owner_user_id", id);
                  const owner = candidates.find((item) => item.id === id);
                  setSelectedLabel(owner?.display_name || owner?.email || id);
                }}
              >
                <SelectTrigger id="destination-owner">
                  <SelectValue placeholder="Select destination owner" />
                </SelectTrigger>
                <SelectContent>
                  {selectedId &&
                    !candidates.some((owner) => owner.id === selectedId) && (
                      <SelectItem value={selectedId}>
                        {selectedLabel}
                      </SelectItem>
                    )}
                  {candidates.map((owner) => (
                    <SelectItem
                      key={owner.id}
                      value={owner.id}
                      disabled={
                        !owner.is_active || owner.id === resource.owner_user_id
                      }
                    >
                      {owner.display_name || owner.email} · {owner.email}
                      {!owner.is_active ? " (inactive)" : ""}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
              {selectedId && (
                <p className="break-all text-xs text-muted-foreground">
                  {selectedId}
                </p>
              )}
              {form.formState.errors.new_owner_user_id && (
                <p role="alert" className="text-sm text-destructive">
                  {form.formState.errors.new_owner_user_id.message}
                </p>
              )}
              {owners.error && (
                <p role="alert" className="text-sm text-destructive">
                  {errorMessage(owners.error)}
                </p>
              )}
              {!owners.isLoading &&
                !owners.error &&
                candidates.length === 0 && (
                  <p className="text-sm text-muted-foreground">
                    No matching owners.
                  </p>
                )}
              <div className="flex items-center justify-between">
                <Button
                  type="button"
                  variant="ghost"
                  disabled={pending || ownerPage === 1}
                  onClick={() => setOwnerPage((page) => page - 1)}
                >
                  Previous owners
                </Button>
                <Button
                  type="button"
                  variant="ghost"
                  disabled={
                    pending || ownerPage * 20 >= (owners.data?.total ?? 0)
                  }
                  onClick={() => setOwnerPage((page) => page + 1)}
                >
                  More owners
                </Button>
              </div>
            </div>
          </form>
        )}
        {error && (
          <p role="alert" className="text-sm text-destructive">
            {error}
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
            {review ? "Back" : "Cancel"}
          </Button>
          {review ? (
            <Button
              type="button"
              disabled={pending || review.preview.blockers.length > 0}
              onClick={() => void confirm()}
            >
              {pending ? "Transferring…" : "Confirm transfer"}
            </Button>
          ) : (
            <Button
              type="submit"
              form="ownership-transfer"
              disabled={pending || !selectedId}
            >
              {pending ? "Preparing…" : "Review transfer"}
            </Button>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

export function AdminOwnershipPage() {
  const user = useAuthStore((state) => state.user);
  const allowed = canAdminWrite(user);
  const [kind, setKind] = useState<OwnershipResourceKind>("service");
  const [search, setSearch] = useState("");
  const [offset, setOffset] = useState(0);
  const [selected, setSelected] = useState<OwnershipResource | null>(null);
  const resources = useOwnershipResources(kind, search, offset, allowed);
  if (!allowed)
    return <p>NyxID admin access is required to transfer ownership.</p>;
  return (
    <div className="space-y-6">
      <PageHeader
        title="Ownership transfers"
        description="Reassign catalog services and channel bots between people and organizations."
      />
      <div className="flex flex-wrap gap-3">
        <Select
          value={kind}
          onValueChange={(value) => {
            setKind(value as OwnershipResourceKind);
            setOffset(0);
            setSelected(null);
          }}
        >
          <SelectTrigger aria-label="Resource type" className="w-52">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="service">Catalog services</SelectItem>
            <SelectItem value="channel_bot">Channel bots</SelectItem>
          </SelectContent>
        </Select>
        <Input
          aria-label="Search resources"
          className="max-w-sm"
          placeholder="Search name, slug or ID"
          value={search}
          onChange={(event) => {
            setSearch(event.target.value);
            setOffset(0);
          }}
        />
      </div>
      {resources.error && (
        <p role="alert" className="text-destructive">
          {errorMessage(resources.error)}
        </p>
      )}
      {resources.isLoading ? (
        <p>Loading resources…</p>
      ) : (
        <div className="overflow-x-auto rounded-md border">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>Resource</TableHead>
                <TableHead>Current owner</TableHead>
                <TableHead className="text-right">Action</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {resources.data?.items.map((resource) => (
                <TableRow key={resource.id}>
                  <TableCell>
                    <p className="font-medium">{resource.name}</p>
                    <p className="text-xs text-muted-foreground">
                      {resource.slug || resource.platform}
                    </p>
                  </TableCell>
                  <TableCell className="break-all font-mono text-xs">
                    {resource.owner_user_id}
                  </TableCell>
                  <TableCell className="text-right">
                    <Button
                      variant="outline"
                      onClick={() => setSelected(resource)}
                    >
                      Transfer ownership
                      <span className="sr-only"> of {resource.name}</span>
                    </Button>
                  </TableCell>
                </TableRow>
              ))}
              {!resources.error && resources.data?.items.length === 0 && (
                <TableRow>
                  <TableCell
                    colSpan={3}
                    className="py-8 text-center text-muted-foreground"
                  >
                    No matching resources.
                  </TableCell>
                </TableRow>
              )}
            </TableBody>
          </Table>
        </div>
      )}
      <div className="flex justify-between">
        <Button
          variant="outline"
          disabled={offset === 0 || resources.isFetching}
          onClick={() => setOffset((current) => Math.max(0, current - 50))}
        >
          Previous
        </Button>
        <Button
          variant="outline"
          disabled={resources.data?.next_offset == null || resources.isFetching}
          onClick={() => setOffset(resources.data?.next_offset ?? offset)}
        >
          Next
        </Button>
      </div>
      {selected && (
        <OwnershipTransferDialog
          key={`${kind}:${selected.id}`}
          kind={kind}
          resource={selected}
          onClose={() => setSelected(null)}
        />
      )}
    </div>
  );
}
