import { useState } from "react";
import { useNavigate } from "@tanstack/react-router";
import { useForm } from "react-hook-form";
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
import { PageHeader } from "@/components/shared/page-header";
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
import { Users, Plus, Trash2 } from "lucide-react";
import { toast } from "sonner";

export function AdminGroupsPage() {
  const navigate = useNavigate();
  const [createOpen, setCreateOpen] = useState(false);
  const [deleteGroupId, setDeleteGroupId] = useState<string | null>(null);

  const { data, isLoading, error } = useGroups();
  const { data: rolesData } = useRoles();
  const createMutation = useCreateGroup();
  const deleteMutation = useDeleteGroup();

  const groups = data?.groups ?? [];
  const availableRoles = rolesData?.roles ?? [];

  const createForm = useForm<CreateGroupFormData>({
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
      toast.success("Group created successfully");
      setCreateOpen(false);
    } catch (err) {
      if (err instanceof ApiError) {
        createForm.setError("root", { message: err.message });
      } else {
        toast.error("Failed to create group");
      }
    }
  }

  async function handleDelete() {
    if (!deleteGroupId) return;
    try {
      await deleteMutation.mutateAsync(deleteGroupId);
      toast.success("Group deleted");
    } catch (err) {
      toast.error(
        err instanceof ApiError ? err.message : "Failed to delete group",
      );
    } finally {
      setDeleteGroupId(null);
    }
  }

  return (
    <div className="space-y-6">
      <PageHeader
        title="Group Management"
        description="Manage groups and their role assignments."
        actions={
          <Button size="sm" onClick={openCreateDialog}>
            <Plus className="mr-1 h-4 w-4" />
            Add Group
          </Button>
        }
      />

      {isLoading ? (
        <div className="space-y-2">
          {Array.from({ length: 5 }).map((_, i) => (
            <Skeleton
              key={`group-skel-${String(i)}`}
              className="h-12 w-full"
            />
          ))}
        </div>
      ) : error ? (
        <div className="flex flex-col items-center justify-center py-12 text-center">
          <Users className="mb-4 h-12 w-12 text-muted-foreground/50" />
          <p className="text-sm text-muted-foreground">
            Failed to load groups. Please try again.
          </p>
        </div>
      ) : groups.length === 0 ? (
        <div className="flex flex-col items-center justify-center py-12 text-center">
          <Users className="mb-4 h-12 w-12 text-muted-foreground/50" />
          <p className="text-sm text-muted-foreground">No groups found.</p>
        </div>
      ) : (
        <div className="rounded-md border">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>Name</TableHead>
                <TableHead>Slug</TableHead>
                <TableHead>Roles</TableHead>
                <TableHead>Members</TableHead>
                <TableHead>Created</TableHead>
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
                  <TableCell className="font-medium">{group.name}</TableCell>
                  <TableCell className="font-mono text-xs">
                    {group.slug}
                  </TableCell>
                  <TableCell>
                    <div className="flex flex-wrap gap-1">
                      {group.roles.length === 0 ? (
                        <span className="text-muted-foreground text-xs">
                          None
                        </span>
                      ) : (
                        group.roles.map((role) => (
                          <Badge
                            key={role.id}
                            variant="outline"
                            className="text-xs"
                          >
                            {role.name}
                          </Badge>
                        ))
                      )}
                    </div>
                  </TableCell>
                  <TableCell>
                    <span className="text-muted-foreground text-xs">
                      {String(group.member_count)}
                    </span>
                  </TableCell>
                  <TableCell className="text-muted-foreground">
                    {formatDate(group.created_at)}
                  </TableCell>
                  <TableCell>
                    <Button
                      variant="ghost"
                      size="icon"
                      className="h-8 w-8"
                      onClick={(e) => {
                        e.stopPropagation();
                        setDeleteGroupId(group.id);
                      }}
                    >
                      <Trash2 className="h-4 w-4 text-muted-foreground" />
                    </Button>
                  </TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        </div>
      )}

      {/* Create Group Dialog */}
      <Dialog open={createOpen} onOpenChange={setCreateOpen}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Create Group</DialogTitle>
            <DialogDescription>
              Create a new group to organize users and inherit roles.
            </DialogDescription>
          </DialogHeader>
          <Form {...createForm}>
            <form
              onSubmit={createForm.handleSubmit((data) =>
                void handleCreate(data),
              )}
              className="space-y-4"
            >
              {createForm.formState.errors.root && (
                <div className="rounded-md bg-destructive/10 p-3 text-sm text-destructive">
                  {createForm.formState.errors.root.message}
                </div>
              )}
              <FormField
                control={createForm.control}
                name="name"
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>Name</FormLabel>
                    <FormControl>
                      <Input placeholder="e.g. Engineering" {...field} />
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
                    <FormLabel>Slug</FormLabel>
                    <FormControl>
                      <Input placeholder="e.g. engineering" {...field} />
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
                    <FormLabel>Description</FormLabel>
                    <FormControl>
                      <Input placeholder="Optional description" {...field} />
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
                    <FormLabel>Roles</FormLabel>
                    <FormControl>
                      <select
                        className="flex h-9 w-full rounded-md border border-input bg-transparent px-3 py-1 text-sm shadow-sm transition-colors focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
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
                >
                  Cancel
                </Button>
                <Button type="submit" isLoading={createMutation.isPending}>
                  Create Group
                </Button>
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
            <DialogTitle>Delete Group</DialogTitle>
            <DialogDescription>
              Are you sure you want to delete this group? All members will be
              removed. This action cannot be undone.
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button variant="outline" onClick={() => setDeleteGroupId(null)}>
              Cancel
            </Button>
            <Button
              variant="destructive"
              onClick={() => void handleDelete()}
              isLoading={deleteMutation.isPending}
            >
              Delete Group
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}
