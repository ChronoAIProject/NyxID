import { useUiTranslation } from "@/i18n/use-ui-translation";
import { translateUi } from "@/i18n/use-ui-translation";
import { translateErrorMessage } from "@/lib/error-messages";
import { useState } from "react";
import { useNavigate } from "@tanstack/react-router";
import { zodResolver } from "@hookform/resolvers/zod";
import {
  useGroups,
  useCreateGroup,
  useDeleteGroup,
  useRoles,
} from "@/hooks/use-rbac";
import { createGroupSchema, type CreateGroupFormData } from "@/schemas/rbac";
import { ApiError } from "@/lib/api-client";
import { formatDate } from "@/lib/utils";
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
  useAppForm,
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
import { Trash2 } from "lucide-react";
import { HierarchyIcon } from "@/components/icons/empty-state";
import { toast } from "sonner";

export function AdminGroupsPage() {
  useUiTranslation();
  const navigate = useNavigate();
  const currentUser = useAuthStore((s) => s.user);
  const canWrite = canAdminWrite(currentUser);
  const [createOpen, setCreateOpen] = useState(false);
  const [deleteGroupId, setDeleteGroupId] = useState<string | null>(null);

  const { data, isLoading, error } = useGroups();
  const { data: rolesData } = useRoles();
  const createMutation = useCreateGroup();
  const deleteMutation = useDeleteGroup();

  const groups = data?.groups ?? [];
  const availableRoles = rolesData?.roles ?? [];

  const createForm = useAppForm<CreateGroupFormData>({
    resolver: zodResolver(createGroupSchema),
    defaultValues: {
      name: "",
      slug: "",
      description: "",
      role_ids: "",
      parent_group_id: "",
    },
  });

  function openCreateDialog() {
    createForm.reset({
      name: "",
      slug: "",
      description: "",
      role_ids: "",
      parent_group_id: "",
    });
    setCreateOpen(true);
  }

  async function handleCreate(data: CreateGroupFormData) {
    try {
      const roleIds = data.role_ids
        ? data.role_ids
            .split(",")
            .map((id) => id.trim())
            .filter((id) => id.length > 0)
        : [];
      await createMutation.mutateAsync({
        name: data.name,
        slug: data.slug,
        description: data.description || undefined,
        role_ids: roleIds,
        parent_group_id: data.parent_group_id || undefined,
      });
      toast.success(translateUi("Group created successfully"));
      setCreateOpen(false);
    } catch (err) {
      toast.error(
        translateErrorMessage(err instanceof ApiError ? err.message : "Failed to create group"),
      );
    }
  }

  async function handleDelete() {
    if (!deleteGroupId) return;
    try {
      await deleteMutation.mutateAsync(deleteGroupId);
      toast.success(translateUi("Group deleted"));
    } catch (err) {
      toast.error(
        translateErrorMessage(err instanceof ApiError ? err.message : "Failed to delete group"),
      );
    } finally {
      setDeleteGroupId(null);
    }
  }

  return (
    <div className="space-y-8">
      <PageHeader
        title={translateUi("Group Management")}
        description={translateUi("Manage groups and their role assignments.")}
        actions={
          canWrite ? (
            <AddCtaButton label={translateUi("Add Group")} onClick={openCreateDialog} />
          ) : null
        }
      />

      {isLoading ? (
        <div className="space-y-2">
          {Array.from({ length: 5 }).map((_, i) => (
            <Skeleton key={`group-skel-${String(i)}`} className="h-12 w-full" />
          ))}
        </div>
      ) : error ? (
        <div className="flex flex-col items-center justify-center gap-1 py-12 text-center">
          <HierarchyIcon className="h-64 w-64 text-muted-foreground" />
          <div className="space-y-1">
            <p className="type-row-title text-muted-foreground">{translateUi("Failed to load groups")}</p>
            <p className="text-13 text-muted-foreground">{translateUi("Please try again later.")}</p>
          </div>
        </div>
      ) : groups.length === 0 ? (
        <div className="flex flex-col items-center justify-center gap-1 py-12 text-center">
          <HierarchyIcon className="h-64 w-64 text-muted-foreground" />
          <div className="space-y-1">
            <p className="type-row-title text-muted-foreground">{translateUi("No groups found")}</p>
            <p className="text-13 text-muted-foreground">{translateUi("There are no groups to display.")}</p>
          </div>
        </div>
      ) : (
        <>
        {/* Mobile cards */}
        <div className="flex flex-col gap-3 md:hidden">
          {groups.map((group) => (
            <div
              key={group.id}
              className="rounded-xl border border-border/50 bg-card p-4 transition-colors hover:bg-overlay cursor-pointer"
              tabIndex={0}
              role="link"
              onClick={() =>
                void navigate({
                  to: "/admin/groups/$groupId",
                  params: { groupId: group.id },
                })
              }
              onKeyDown={(e) => {
                if (e.key === "Enter" || e.key === " ") {
                  e.preventDefault();
                  void navigate({
                    to: "/admin/groups/$groupId",
                    params: { groupId: group.id },
                  });
                }
              }}
            >
              <div className="flex items-start justify-between gap-2">
                <div className="min-w-0 flex-1">
                  <p className="type-row-title truncate">{group.name}</p>
                  <p className="text-13 text-muted-foreground font-mono truncate mt-0.5">
                    {group.slug}
                  </p>
                </div>
                <Badge variant="secondary" className="text-12">
                  {String(group.member_count)}{translateUi(" member")}{group.member_count !== 1 ? "s" : ""}
                </Badge>
              </div>
              <div className="mt-3 flex items-center gap-3 text-13 text-muted-foreground">
                <span>{translateUi("Created ")}{formatDate(group.created_at)}</span>
              </div>
            </div>
          ))}
        </div>

        {/* Desktop table */}
        <div className="hidden md:block rounded-xl border border-border/50 bg-card overflow-hidden">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>{translateUi("Name")}</TableHead>
                <TableHead>{translateUi("Slug")}</TableHead>
                <TableHead>{translateUi("Roles")}</TableHead>
                <TableHead>{translateUi("Members")}</TableHead>
                <TableHead>{translateUi("Created")}</TableHead>
                <TableHead className="w-[60px]" />
              </TableRow>
            </TableHeader>
            <TableBody>
              {groups.map((group) => (
                <TableRow
                  key={group.id}
                  className="cursor-pointer"
                  tabIndex={0}
                  role="link"
                  onClick={() =>
                    void navigate({
                      to: "/admin/groups/$groupId",
                      params: { groupId: group.id },
                    })
                  }
                  onKeyDown={(e) => {
                    if (e.key === "Enter" || e.key === " ") {
                      e.preventDefault();
                      void navigate({
                        to: "/admin/groups/$groupId",
                        params: { groupId: group.id },
                      });
                    }
                  }}
                >
                  <TableCell className="font-normal">{group.name}</TableCell>
                  <TableCell className="font-mono text-14">
                    {group.slug}
                  </TableCell>
                  <TableCell>
                    <div className="flex flex-wrap gap-1">
                      {group.roles.length === 0 ? (
                        <span className="text-muted-foreground text-13">{translateUi("None")}</span>
                      ) : (
                        group.roles.map((role) => (
                          <Badge
                            key={role.id}
                            variant="secondary"
                            className="text-12"
                          >
                            {role.name}
                          </Badge>
                        ))
                      )}
                    </div>
                  </TableCell>
                  <TableCell>
                    <span className="text-muted-foreground text-13">
                      {String(group.member_count)}
                    </span>
                  </TableCell>
                  <TableCell className="text-muted-foreground">
                    {formatDate(group.created_at)}
                  </TableCell>
                  <TableCell>
                    {canWrite && (
                      <Button
                        variant="ghost"
                        size="icon"
                        className="h-8 w-8"
                        onClick={(e) => {
                          e.stopPropagation();
                          setDeleteGroupId(group.id);
                        }}
                      >
                        <Trash2 className="h-3 w-3 text-muted-foreground" />
                      </Button>
                    )}
                  </TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        </div>
        </>
      )}

      {/* Create Group Dialog */}
      <Dialog open={createOpen} onOpenChange={setCreateOpen}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>{translateUi("Create Group")}</DialogTitle>
            <DialogDescription>{translateUi("Create a new group to organize users and inherit roles.")}</DialogDescription>
          </DialogHeader>
          <Form {...createForm}>
            <form
              onSubmit={createForm.handleSubmit(
                (data) => void handleCreate(data),
              )}
              className="space-y-4"
            >
              <FormField
                control={createForm.control}
                name="name"
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>{translateUi("Name")}</FormLabel>
                    <FormControl>
                      <Input placeholder={translateUi("e.g. Engineering")} {...field} />
                    </FormControl>
                    <FormMessage />
                  </FormItem>
                )}
              />
              <FormField
                control={createForm.control}
                name="slug"
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>{translateUi("Slug")}</FormLabel>
                    <FormControl>
                      <Input placeholder={translateUi("e.g. engineering")} {...field} />
                    </FormControl>
                    <FormMessage />
                  </FormItem>
                )}
              />
              <FormField
                control={createForm.control}
                name="description"
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>{translateUi("Description")}</FormLabel>
                    <FormControl>
                      <Input placeholder={translateUi("Optional description")} {...field} />
                    </FormControl>
                    <FormMessage />
                  </FormItem>
                )}
              />
              <FormField
                control={createForm.control}
                name="role_ids"
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>{translateUi("Roles")}</FormLabel>
                    <FormControl>
                      <select
                        className="flex w-full rounded-lg border border-input bg-popover px-3 py-1.5 text-14 text-foreground shadow-sm transition-colors focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring [&_option]:bg-popover [&_option]:text-foreground [&_option:checked]:bg-primary/20"
                        multiple
                        value={
                          field.value
                            ? field.value
                                .split(",")
                                .map((s) => s.trim())
                                .filter(Boolean)
                            : []
                        }
                        onChange={(e) => {
                          const selected = Array.from(
                            e.target.selectedOptions,
                            (o) => o.value,
                          );
                          field.onChange(selected.join(","));
                        }}
                        style={{ minHeight: "80px" }}
                      >
                        {availableRoles.map((role) => (
                          <option key={role.id} value={role.id}>
                            {role.name} ({role.slug})
                          </option>
                        ))}
                      </select>
                    </FormControl>
                    <FormMessage />
                  </FormItem>
                )}
              />
              <DialogFooter>
                <Button
                  type="button"
                  variant="outline"
                  onClick={() => setCreateOpen(false)}
                >{translateUi("Cancel")}</Button>
                <Button type="submit" variant="primary" isLoading={createMutation.isPending}>{translateUi("Create Group")}</Button>
              </DialogFooter>
            </form>
          </Form>
        </DialogContent>
      </Dialog>

      {/* Delete Confirmation Dialog */}
      <Dialog
        open={deleteGroupId !== null}
        onOpenChange={(open) => {
          if (!open) setDeleteGroupId(null);
        }}
      >
        <DialogContent>
          <DialogHeader>
            <DialogTitle>{translateUi("Delete Group")}</DialogTitle>
            <DialogDescription>{translateUi("Are you sure you want to delete this group? All members will be removed. This action cannot be undone.")}</DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button variant="outline" onClick={() => setDeleteGroupId(null)}>{translateUi("Cancel")}</Button>
            <Button
              variant="destructive"
              onClick={() => void handleDelete()}
              isLoading={deleteMutation.isPending}
            >{translateUi("Delete Group")}</Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}
