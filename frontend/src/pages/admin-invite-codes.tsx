import { useEffect, useRef, useState } from "react";
import { useForm } from "react-hook-form";
import { zodResolver } from "@hookform/resolvers/zod";
import {
  useAdminInviteCodes,
  useCreateInviteCode,
  useDeactivateInviteCode,
  useUpdateInviteCode,
} from "@/hooks/use-admin-invite-codes";
import {
  createInviteCodeSchema,
  type CreateInviteCodeFormData,
} from "@/schemas/admin";
import { ApiError } from "@/lib/api-client";
import { cn, copyToClipboard, formatDate } from "@/lib/utils";
import { canAdminWrite } from "@/types/api";
import { useAuthStore } from "@/stores/auth-store";
import { PageHeader } from "@/components/shared/page-header";
import { AddCtaButton } from "@/components/shared/add-cta-button";
import { Skeleton } from "@/components/ui/skeleton";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Input } from "@/components/ui/input";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import {
  Sheet,
  SheetContent,
  SheetDescription,
  SheetHeader,
  SheetTitle,
} from "@/components/ui/sheet";
import {
  Form,
  FormControl,
  FormField,
  FormItem,
  FormLabel,
  FormMessage,
} from "@/components/ui/form";
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
import { Copy, Check, Ban, Link2, MoreVertical } from "lucide-react";
import { MysteryBoxIcon } from "@/components/icons/empty-state";
import { toast } from "sonner";
import type { InviteCode } from "@/types/admin";

const DEFAULT_MAX_USES = 10;

export function AdminInviteCodesPage() {
  const currentUser = useAuthStore((s) => s.user);
  const canWrite = canAdminWrite(currentUser);
  const { data, isLoading, error } = useAdminInviteCodes();
  const createMutation = useCreateInviteCode();
  const deactivateMutation = useDeactivateInviteCode();
  const updateMutation = useUpdateInviteCode();

  const [createOpen, setCreateOpen] = useState(false);
  const [createdCode, setCreatedCode] = useState<InviteCode | null>(null);
  const [copiedId, setCopiedId] = useState<string | null>(null);
  const [, setCopiedLinkId] = useState<string | null>(null);
  const [deactivateTarget, setDeactivateTarget] = useState<InviteCode | null>(
    null,
  );
  // Track the selected row by id (not the full object) so that the drawer
  // always reflects the freshest data after query invalidations.
  const [selectedCodeId, setSelectedCodeId] = useState<string | null>(null);
  const [noteDraft, setNoteDraft] = useState("");
  // The note value we last loaded into the draft. Compared against `noteDraft`
  // to compute `noteHasChanges`, so the dirty signal reflects "the user typed
  // something" rather than "the live persisted value moved out from under us
  // due to a background refetch." Without this ref, a window-focus refetch
  // could silently mark the drawer dirty and let the admin clobber another
  // admin's update with a value they never typed.
  const lastSyncedNoteRef = useRef<string>("");

  const inviteCodes = data?.invite_codes ?? [];
  const selectedCode =
    selectedCodeId !== null
      ? (inviteCodes.find((ic) => ic.id === selectedCodeId) ?? null)
      : null;
  const noteHasChanges =
    selectedCode !== null && noteDraft !== lastSyncedNoteRef.current;
  // Single source of truth for "save in flight". Every interaction that could
  // change `selectedCodeId` or submit another PATCH is gated on this flag, so
  // the cross-row race and double-submit windows are closed at the UI level
  // rather than defensively patched in the handler.
  const isSaving = updateMutation.isPending;

  // Reset the draft only when the user opens a different row — not on every
  // background refetch, otherwise React Query's default refetchOnWindowFocus
  // would wipe an in-progress edit when the user tabs away and back.
  useEffect(() => {
    const note = selectedCode?.note ?? "";
    setNoteDraft(note);
    lastSyncedNoteRef.current = note;
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [selectedCodeId]);

  const createForm = useForm<CreateInviteCodeFormData>({
    resolver: zodResolver(createInviteCodeSchema),
    defaultValues: {
      max_uses: DEFAULT_MAX_USES,
      note: "",
    },
  });

  function openCreateDialog() {
    createForm.reset({
      max_uses: DEFAULT_MAX_USES,
      note: "",
    });
    setCreatedCode(null);
    setCreateOpen(true);
  }

  async function handleCreate(formData: CreateInviteCodeFormData) {
    try {
      const created = await createMutation.mutateAsync({
        max_uses: formData.max_uses,
        note: formData.note || undefined,
      });
      setCreatedCode(created);
      toast.success("Invite code created");
    } catch (err) {
      toast.error(
        err instanceof ApiError ? err.message : "Failed to create invite code",
      );
    }
  }

  async function handleCopyCode(code: string, id: string) {
    try {
      await copyToClipboard(code);
      setCopiedId(id);
      toast.success("Code copied to clipboard");
      setTimeout(() => {
        setCopiedId((current) => (current === id ? null : current));
      }, 2000);
    } catch {
      toast.error("Failed to copy to clipboard");
    }
  }

  async function handleCopyLink(code: string, id: string) {
    try {
      const link = `${window.location.origin}/register?code=${encodeURIComponent(code)}`;
      await copyToClipboard(link);
      setCopiedLinkId(id);
      toast.success("Invite link copied to clipboard");
      setTimeout(() => {
        setCopiedLinkId((current) => (current === id ? null : current));
      }, 2000);
    } catch {
      toast.error("Failed to copy to clipboard");
    }
  }

  async function handleDeactivate() {
    if (!deactivateTarget) return;
    try {
      await deactivateMutation.mutateAsync(deactivateTarget.id);
      toast.success(`Invite code ${deactivateTarget.code} deactivated`);
    } catch (err) {
      toast.error(
        err instanceof ApiError ? err.message : "Failed to deactivate code",
      );
    } finally {
      setDeactivateTarget(null);
    }
  }

  async function handleSaveNote() {
    if (!selectedCode || !noteHasChanges) return;
    // Capture the id at call time. Even though the row-click / drawer-close
    // guards below make `selectedCodeId` immutable while `isSaving === true`,
    // keeping an explicit local + post-await equality check means a future
    // refactor that relaxes those guards can't silently reintroduce the
    // cross-row race (ref gets written for the wrong code after navigation).
    const savingId = selectedCode.id;
    try {
      const updated = await updateMutation.mutateAsync({
        id: savingId,
        body: { note: noteDraft },
      });
      if (selectedCodeId === savingId) {
        // Sync the ref so noteHasChanges flips false now that the saved value
        // is the new baseline. Without this the Save button would stay enabled
        // because the ref still points at the value we loaded when the drawer
        // first opened.
        lastSyncedNoteRef.current = updated.note ?? "";
      }
      toast.success("Note updated");
    } catch (err) {
      if (err instanceof DOMException && err.name === "AbortError") {
        toast.error("Save timed out after 10 seconds. Try again.");
      } else if (err instanceof ApiError) {
        toast.error(err.message);
      } else {
        toast.error("Failed to update note");
      }
    }
  }

  function getStatusBadge(ic: InviteCode) {
    const exhausted = ic.used_count >= ic.max_uses;
    if (!ic.is_active) return <Badge variant="destructive">Deactivated</Badge>;
    if (exhausted) return <Badge variant="warning">Exhausted</Badge>;
    return <Badge variant="success">Active</Badge>;
  }

  return (
    <div className="space-y-8">
      <PageHeader
        title="Invite Codes"
        description="Create and manage invite codes that gate new user registration. Each code can grant a bounded number of registrations and can be deactivated at any time."
        actions={
          canWrite ? (
            <AddCtaButton label="Create Invite Code" onClick={openCreateDialog} />
          ) : null
        }
      />

      {isLoading ? (
        <div className="space-y-2">
          {Array.from({ length: 4 }).map((_, i) => (
            <Skeleton
              key={`invite-skel-${String(i)}`}
              className="h-12 w-full"
            />
          ))}
        </div>
      ) : error ? (
        <div className="flex flex-col items-center justify-center gap-1 py-12 text-center">
          <MysteryBoxIcon className="h-64 w-64 text-muted-foreground/30" />
          <div className="space-y-1">
            <p className="text-[12px] font-medium text-muted-foreground/30">Failed to load invite codes</p>
            <p className="text-xs text-muted-foreground/30">Please try again later.</p>
          </div>
        </div>
      ) : inviteCodes.length === 0 ? (
        <div className="flex flex-col items-center justify-center gap-1 py-12 text-center">
          <MysteryBoxIcon className="h-64 w-64 text-muted-foreground/30" />
          <div className="space-y-1">
            <p className="text-[12px] font-medium text-muted-foreground/30">No invite codes found</p>
            <p className="text-xs text-muted-foreground/30">Create one to allow a new user to register.</p>
          </div>
        </div>
      ) : (
        <>
        {/* Mobile cards */}
        <div className="flex flex-col gap-3 md:hidden">
          {inviteCodes.map((ic) => (
            <div
              key={ic.id}
              className={cn(
                "rounded-xl border border-border/50 bg-card p-4 transition-colors hover:bg-white/[0.03] cursor-pointer",
                isSaving && "pointer-events-none opacity-60",
              )}
              onClick={() => {
                if (isSaving) return;
                setSelectedCodeId(ic.id);
              }}
            >
              <div className="flex items-start justify-between gap-2">
                <div className="min-w-0 flex-1">
                  <p className="text-sm font-medium font-mono text-foreground truncate">
                    {ic.code}
                  </p>
                </div>
                <div className="flex items-center gap-1.5 shrink-0">
                  {getStatusBadge(ic)}
                  <DropdownMenu>
                    <DropdownMenuTrigger asChild>
                      <Button
                        variant="ghost"
                        size="icon"
                        className="h-7 w-7 text-muted-foreground hover:text-foreground"
                        onClick={(e) => e.stopPropagation()}
                      >
                        <MoreVertical className="h-3 w-3" />
                      </Button>
                    </DropdownMenuTrigger>
                    <DropdownMenuContent align="end">
                      <DropdownMenuItem
                        onClick={(e) => {
                          e.stopPropagation();
                          void handleCopyCode(ic.code, ic.id);
                        }}
                      >
                        <Copy className="h-3 w-3" />
                        Copy code
                      </DropdownMenuItem>
                      <DropdownMenuItem
                        onClick={(e) => {
                          e.stopPropagation();
                          void handleCopyLink(ic.code, ic.id);
                        }}
                      >
                        <Link2 className="h-3 w-3" />
                        Copy invite link
                      </DropdownMenuItem>
                      {canWrite && ic.is_active && (
                        <DropdownMenuItem
                          className="text-destructive focus:text-destructive"
                          onClick={(e) => {
                            e.stopPropagation();
                            setDeactivateTarget(ic);
                          }}
                        >
                          <Ban className="h-3 w-3" />
                          Deactivate
                        </DropdownMenuItem>
                      )}
                    </DropdownMenuContent>
                  </DropdownMenu>
                </div>
              </div>
              <div className="mt-2 text-xs text-muted-foreground">
                <span className="tabular-nums">
                  {String(ic.used_count)}/{String(ic.max_uses)} uses
                </span>
                {ic.note && (
                  <>
                    <span className="text-border mx-1.5">|</span>
                    <span className="truncate">{ic.note}</span>
                  </>
                )}
              </div>
              <div className="mt-1.5 text-xs text-muted-foreground">
                <span>Created {formatDate(ic.created_at)}</span>
              </div>
            </div>
          ))}
        </div>

        {/* Desktop table */}
        <div className="hidden md:block rounded-xl border border-border/50 bg-card overflow-hidden">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>Code</TableHead>
                <TableHead>Uses</TableHead>
                <TableHead>Status</TableHead>
                <TableHead>Note</TableHead>
                <TableHead>Created</TableHead>
                <TableHead className="w-[100px]">Actions</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {inviteCodes.map((ic) => (
                <TableRow
                  key={ic.id}
                  onClick={() => {
                    if (isSaving) return;
                    setSelectedCodeId(ic.id);
                  }}
                  className={cn(
                    "cursor-pointer",
                    isSaving && "pointer-events-none opacity-60",
                  )}
                >
                  <TableCell>
                    <span className="font-mono text-sm font-medium text-foreground">
                      {ic.code}
                    </span>
                  </TableCell>
                  <TableCell>
                    <span className="text-sm tabular-nums text-muted-foreground">
                      {String(ic.used_count)}/{String(ic.max_uses)}
                    </span>
                  </TableCell>
                  <TableCell>{getStatusBadge(ic)}</TableCell>
                  <TableCell>
                    {ic.note ? (
                      <span className="text-[11px] text-text-tertiary">
                        {ic.note}
                      </span>
                    ) : (
                      <span className="text-muted-foreground">--</span>
                    )}
                  </TableCell>
                  <TableCell className="text-muted-foreground">
                    {formatDate(ic.created_at)}
                  </TableCell>
                  <TableCell>
                    <DropdownMenu>
                      <DropdownMenuTrigger asChild>
                        <Button
                          variant="ghost"
                          size="icon"
                          className="h-8 w-8 text-muted-foreground hover:text-foreground"
                          onClick={(e) => e.stopPropagation()}
                        >
                          <MoreVertical className="h-3 w-3" />
                        </Button>
                      </DropdownMenuTrigger>
                      <DropdownMenuContent align="end">
                        <DropdownMenuItem
                          onClick={(e) => {
                            e.stopPropagation();
                            void handleCopyCode(ic.code, ic.id);
                          }}
                        >
                          <Copy className="h-3 w-3" />
                          Copy code
                        </DropdownMenuItem>
                        <DropdownMenuItem
                          onClick={(e) => {
                            e.stopPropagation();
                            void handleCopyLink(ic.code, ic.id);
                          }}
                        >
                          <Link2 className="h-3 w-3" />
                          Copy invite link
                        </DropdownMenuItem>
                        {canWrite && ic.is_active && (
                          <DropdownMenuItem
                            className="text-destructive focus:text-destructive"
                            onClick={(e) => {
                              e.stopPropagation();
                              setDeactivateTarget(ic);
                            }}
                          >
                            <Ban className="h-3 w-3" />
                            Deactivate
                          </DropdownMenuItem>
                        )}
                      </DropdownMenuContent>
                    </DropdownMenu>
                  </TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        </div>
        </>
      )}

      {/* Create Invite Code Dialog */}
      <Dialog
        open={createOpen}
        onOpenChange={(open) => {
          setCreateOpen(open);
          if (!open) {
            setCreatedCode(null);
          }
        }}
      >
        <DialogContent>
          <DialogHeader>
            <DialogTitle>
              {createdCode ? "Invite code created" : "Create Invite Code"}
            </DialogTitle>
            <DialogDescription>
              {createdCode
                ? "Share the code below with the user who should register. The code is also visible in the table."
                : "Generate a new invite code. The code is created server-side and shown once it is ready."}
            </DialogDescription>
          </DialogHeader>

          {createdCode ? (
            <div className="space-y-4">
              <div className="rounded-md border border-border bg-muted/30 p-4">
                <p className="mb-2 text-xs uppercase tracking-wider text-text-tertiary">
                  Invite code
                </p>
                <div className="flex items-center justify-between gap-2">
                  <span className="font-mono text-lg font-medium text-foreground">
                    {createdCode.code}
                  </span>
                  <Button
                    variant="outline"
                    size="sm"
                    onClick={() =>
                      void handleCopyCode(createdCode.code, createdCode.id)
                    }
                  >
                    {copiedId === createdCode.id ? (
                      <>
                        <Check
                          className="h-3 w-3 text-success"
                          aria-hidden="true"
                        />
                        Copied
                      </>
                    ) : (
                      <>
                        <Copy className="h-3 w-3" aria-hidden="true" />
                        Copy
                      </>
                    )}
                  </Button>
                </div>
                <p className="mt-2 text-xs text-muted-foreground">
                  Up to {String(createdCode.max_uses)} registrations.
                </p>
              </div>
              <DialogFooter>
                <Button onClick={() => setCreateOpen(false)}>Done</Button>
              </DialogFooter>
            </div>
          ) : (
            <Form {...createForm}>
              <form
                onSubmit={createForm.handleSubmit(
                  (formData) => void handleCreate(formData),
                )}
                className="space-y-4"
              >
                <FormField
                  control={createForm.control}
                  name="max_uses"
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>Max uses</FormLabel>
                      <FormControl>
                        <Input
                          type="number"
                          min={1}
                          max={1000}
                          step={1}
                          value={String(field.value)}
                          onChange={(e) =>
                            field.onChange(e.target.valueAsNumber)
                          }
                          onBlur={field.onBlur}
                          name={field.name}
                          ref={field.ref}
                        />
                      </FormControl>
                      <p className="text-xs text-muted-foreground">
                        Maximum number of registrations this code can grant
                        (1-1000).
                      </p>
                      <FormMessage />
                    </FormItem>
                  )}
                />
                <FormField
                  control={createForm.control}
                  name="note"
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>Note</FormLabel>
                      <FormControl>
                        <Input
                          placeholder="e.g. alice@corp"
                          maxLength={512}
                          {...field}
                        />
                      </FormControl>
                      <p className="text-xs text-muted-foreground">
                        Optional reminder of who this code is for. Visible to
                        admins only.
                      </p>
                      <FormMessage />
                    </FormItem>
                  )}
                />
                <DialogFooter>
                  <Button
                    type="button"
                    variant="outline"
                    onClick={() => setCreateOpen(false)}
                  >
                    Cancel
                  </Button>
                  <Button type="submit" variant="primary" isLoading={createMutation.isPending}>
                    Create Invite Code
                  </Button>
                </DialogFooter>
              </form>
            </Form>
          )}
        </DialogContent>
      </Dialog>

      {/* Deactivate Confirmation Dialog */}
      <Dialog
        open={deactivateTarget !== null}
        onOpenChange={(open) => {
          if (!open) setDeactivateTarget(null);
        }}
      >
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Deactivate invite code</DialogTitle>
            <DialogDescription>
              {deactivateTarget
                ? `Deactivating ${deactivateTarget.code} immediately blocks any further registrations with it. This cannot be undone — mint a new code if a user needs another attempt.`
                : ""}
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button
              variant="outline"
              onClick={() => setDeactivateTarget(null)}
            >
              Cancel
            </Button>
            <Button
              variant="destructive"
              onClick={() => void handleDeactivate()}
              isLoading={deactivateMutation.isPending}
            >
              Deactivate
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      {/* Invite Code Detail Drawer */}
      <Sheet
        open={selectedCode !== null}
        onOpenChange={(open) => {
          // Never let a close interaction land while a PATCH is in flight.
          // Combined with the row-click guard above, this makes the cross-row
          // save race structurally impossible: `selectedCodeId` cannot change
          // between `mutateAsync` call and resolution.
          if (!open && !isSaving) setSelectedCodeId(null);
        }}
      >
        <SheetContent
          className="flex w-full flex-col gap-6 overflow-y-auto sm:max-w-lg"
          onPointerDownOutside={(e) => {
            if (isSaving) e.preventDefault();
          }}
          onEscapeKeyDown={(e) => {
            if (isSaving) e.preventDefault();
          }}
        >
          {selectedCode && (
            <>
              <SheetHeader>
                <div className="flex items-center gap-3">
                  <SheetTitle className="font-mono">
                    {selectedCode.code}
                  </SheetTitle>
                  {getStatusBadge(selectedCode)}
                </div>
                <SheetDescription>
                  Detailed usage and editable note for this invite code.
                </SheetDescription>
              </SheetHeader>

              <div className="grid grid-cols-2 gap-4 rounded-md border border-border bg-muted/20 p-4 text-sm">
                <div>
                  <p className="text-xs uppercase tracking-wider text-text-tertiary">
                    Uses
                  </p>
                  <p className="mt-1 tabular-nums">
                    {String(selectedCode.used_count)}/
                    {String(selectedCode.max_uses)}
                  </p>
                </div>
                <div>
                  <p className="text-xs uppercase tracking-wider text-text-tertiary">
                    Created
                  </p>
                  <p className="mt-1 text-muted-foreground">
                    {formatDate(selectedCode.created_at)}
                  </p>
                </div>
                <div className="col-span-2">
                  <p className="text-xs uppercase tracking-wider text-text-tertiary">
                    Created by
                  </p>
                  {(() => {
                    // Mirror the Redemptions list fallback chain: display_name →
                    // email → UUID. Use truthy checks + optional chaining so a
                    // legacy backend that omits the creator sidecar (null or
                    // absent) degrades to the mono UUID rather than rendering
                    // a blank line.
                    const creator = selectedCode.creator;
                    const primary =
                      creator?.display_name ||
                      creator?.email ||
                      selectedCode.created_by;
                    const showEmailLine =
                      !!creator?.display_name && !!creator.email;
                    const isUuidFallback = !creator;
                    return (
                      <>
                        <p
                          className={cn(
                            "mt-1 truncate text-sm text-foreground",
                            isUuidFallback &&
                              "font-mono text-xs text-muted-foreground",
                          )}
                        >
                          {primary}
                        </p>
                        {showEmailLine && (
                          <p className="truncate text-xs text-muted-foreground">
                            {creator.email}
                          </p>
                        )}
                      </>
                    );
                  })()}
                </div>
              </div>

              <div className="space-y-2">
                <label
                  htmlFor="invite-code-note"
                  className="text-sm font-medium"
                >
                  Note
                </label>
                <Input
                  id="invite-code-note"
                  value={noteDraft}
                  onChange={(e) => setNoteDraft(e.target.value)}
                  placeholder="e.g. alice@corp"
                  maxLength={512}
                  readOnly={!canWrite}
                />
                <div className="flex items-center justify-between">
                  <p className="text-xs text-muted-foreground">
                    {canWrite
                      ? "Visible to admins only. Leave blank to clear."
                      : "Read-only — only admins can edit notes."}
                  </p>
                  {canWrite && (
                    <Button
                      size="sm"
                      onClick={() => void handleSaveNote()}
                      disabled={!noteHasChanges || isSaving}
                      isLoading={isSaving}
                    >
                      Save
                    </Button>
                  )}
                </div>
              </div>

              <div className="space-y-3">
                <div className="flex items-baseline justify-between">
                  <h3 className="text-sm font-medium">Redemptions</h3>
                  <span className="text-xs text-muted-foreground">
                    {selectedCode.usages.length} total
                  </span>
                </div>
                {selectedCode.usages.length === 0 ? (
                  <div className="rounded-md border border-dashed border-border py-8 text-center text-[11px] text-text-tertiary">
                    No redemptions yet.
                  </div>
                ) : (
                  <ul className="divide-y divide-border rounded-md border border-border">
                    {selectedCode.usages.map((usage) => {
                      const primary =
                        usage.user_display_name ??
                        usage.user_email ??
                        usage.user_id;
                      const showEmailLine =
                        usage.user_display_name !== null &&
                        usage.user_email !== null;
                      return (
                        <li
                          key={`${usage.user_id}-${usage.used_at}`}
                          className="flex items-start justify-between gap-4 px-3 py-2"
                        >
                          <div className="min-w-0 flex-1">
                            <p className="truncate text-sm text-foreground">
                              {primary}
                            </p>
                            {showEmailLine && (
                              <p className="truncate text-xs text-muted-foreground">
                                {usage.user_email}
                              </p>
                            )}
                            {usage.user_email === null &&
                              usage.user_display_name === null && (
                                <p className="truncate font-mono text-xs text-muted-foreground">
                                  account deleted
                                </p>
                              )}
                          </div>
                          <span className="shrink-0 text-xs text-muted-foreground">
                            {formatDate(usage.used_at)}
                          </span>
                        </li>
                      );
                    })}
                  </ul>
                )}
              </div>
            </>
          )}
        </SheetContent>
      </Sheet>
    </div>
  );
}
