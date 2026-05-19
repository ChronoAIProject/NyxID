import { useState } from "react";
import { useNavigate, useParams } from "@tanstack/react-router";
import { useForm } from "react-hook-form";
import { zodResolver } from "@hookform/resolvers/zod";
import {
  useRole,
  useUpdateRole,
  useDeleteRole,
  useBulkAssignRole,
} from "@/hooks/use-rbac";
import { updateRoleSchema, type UpdateRoleFormData } from "@/schemas/rbac";
import { ApiError } from "@/lib/api-client";
import { formatDate } from "@/lib/utils";
import { canAdminWrite } from "@/types/api";
import { useAuthStore } from "@/stores/auth-store";
import { PageHeader } from "@/components/shared/page-header";
import { useBreadcrumbLabel } from "@/components/layout/dashboard-layout";
import { DetailSection } from "@/components/shared/detail-section";
import { DetailRow } from "@/components/shared/detail-row";
import { Skeleton } from "@/components/ui/skeleton";
import { Button, ButtonIcon } from "@/components/ui/button";
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
import { Pencil, Trash2, AlertCircle, Users } from "lucide-react";
import { SmartRemoteIcon } from "@/components/icons/empty-state";
import { toast } from "sonner";
import { Checkbox } from "@/components/ui/checkbox";

export function AdminRoleDetailPage() {
  const { roleId } = useParams({ strict: false }) as { roleId: string };
  const navigate = useNavigate();
  const currentUser = useAuthStore((s) => s.user);
  const canWrite = canAdminWrite(currentUser);

  const { data: role, isLoading, error } = useRole(roleId);
  const updateMutation = useUpdateRole();
  const deleteMutation = useDeleteRole();
  const bulkAssignMutation = useBulkAssignRole();

  const [editOpen, setEditOpen] = useState(false);
  const [deleteOpen, setDeleteOpen] = useState(false);
  const [bulkAssignOpen, setBulkAssignOpen] = useState(false);

  useBreadcrumbLabel(role?.name);

  const form = useForm<UpdateRoleFormData>({
    resolver: zodResolver(updateRoleSchema),
    defaultValues: {
      name: "",
      slug: "",
      description: "",
      permissions: "",
      is_default: false,
    },
  });

  function openEditDialog() {
    if (!role) return;
    form.reset({
      name: role.name,
      slug: role.slug,
      description: role.description ?? "",
      permissions: role.permissions.join(", "),
      is_default: role.is_default,
    });
    setEditOpen(true);
  }

  async function handleEdit(data: UpdateRoleFormData) {
    try {
      const permissions = data.permissions
        ? data.permissions
            .split(",")
            .map((p) => p.trim())
            .filter((p) => p.length > 0)
        : [];
      await updateMutation.mutateAsync({
        roleId,
        data: {
          name: data.name,
          slug: data.slug,
          description: data.description || undefined,
          permissions,
          is_default: data.is_default,
        },
      });
      toast.success("Role updated successfully");
      setEditOpen(false);
    } catch (err) {
      toast.error(
        err instanceof ApiError ? err.message : "Failed to update role",
      );
    }
  }

  async function handleDelete() {
    try {
      await deleteMutation.mutateAsync(roleId);
      toast.success("Role deleted");
      void navigate({ to: "/admin/roles" });
    } catch (err) {
      toast.error(
        err instanceof ApiError ? err.message : "Failed to delete role",
      );
    } finally {
      setDeleteOpen(false);
    }
  }

  async function handleBulkAssignAll() {
    try {
      const result = await bulkAssignMutation.mutateAsync({
        roleId,
        data: { all: true },
      });
      toast.success(result.message);
      setBulkAssignOpen(false);
    } catch (err) {
      toast.error(
        err instanceof ApiError ? err.message : "Failed to bulk assign role",
      );
    }
  }

  if (isLoading) {
    return (
      <div className="space-y-6">
        <Skeleton className="h-10 w-64" />
        <Skeleton className="h-64 w-full" />
      </div>
    );
  }

  if (error || !role) {
    return (
      <div className="flex flex-col items-center justify-center py-16 text-center">
        <AlertCircle className="mb-4 h-12 w-12 text-muted-foreground/50" />
        <h3 className="mb-2 font-display text-lg font-semibold">
          Role not found
        </h3>
        <p className="mb-4 text-sm text-muted-foreground">
          The role you are looking for does not exist or has been deleted.
        </p>
        <Button
          variant="outline"
          onClick={() => void navigate({ to: "/admin/roles" })}
        >
          Back to Roles
        </Button>
      </div>
    );
  }

  return (
    <div className="space-y-8">
      <PageHeader
        title={role.name}
        description={role.description ?? undefined}
        actions={
          canWrite ? (
            <>
              <Button
                variant="outline"
                onClick={() => setBulkAssignOpen(true)}
              >
                <ButtonIcon><Users className="h-3 w-3" /></ButtonIcon>
                Assign All
              </Button>
              <Button variant="outline" onClick={openEditDialog}>
                <ButtonIcon><Pencil className="h-3 w-3" /></ButtonIcon>
                Edit
              </Button>
              {!role.is_system && (
                <Button
                  variant="destructive"
                  onClick={() => setDeleteOpen(true)}
                >
                  <ButtonIcon variant="destructive"><Trash2 className="h-3 w-3 text-destructive" /></ButtonIcon>
                  Delete
                </Button>
              )}
            </>
          ) : null
        }
      />

      <DetailSection title="Role Information">
        <DetailRow label="ID" value={role.id} copyable />
        <DetailRow label="Name" value={role.name} />
        <DetailRow label="Slug" value={role.slug} copyable />
        <DetailRow
          label="Type"
          value={role.is_system ? "System" : "Custom"}
          badge
          badgeVariant={role.is_system ? "secondary" : "default"}
        />
        <DetailRow
          label="Default"
          value={role.is_default ? "Yes" : "No"}
          badge
          badgeVariant={role.is_default ? "success" : "secondary"}
        />
        {role.client_id && (
          <DetailRow label="Client ID" value={role.client_id} copyable />
        )}
        <DetailRow label="Created" value={formatDate(role.created_at)} />
        <DetailRow label="Updated" value={formatDate(role.updated_at)} />
      </DetailSection>

      <DetailSection title="Permissions">
        {role.permissions.length === 0 ? (
          <div className="flex flex-col items-center justify-center gap-1 py-8 text-center">
            <SmartRemoteIcon className="h-48 w-48 text-muted-foreground/30" />
            <p className="text-[12px] text-muted-foreground/30">No permissions assigned.</p>
          </div>
        ) : (
          <div className="flex flex-wrap gap-2 px-4 py-3">
            {role.permissions.map((perm) => (
              <Badge key={perm} variant="secondary" className="font-mono text-xs">
                {perm}
              </Badge>
            ))}
          </div>
        )}
      </DetailSection>

      {/* Edit Dialog */}
      <Dialog open={editOpen} onOpenChange={setEditOpen}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Edit Role</DialogTitle>
            <DialogDescription>
              Update role details for {role.name}.
            </DialogDescription>
          </DialogHeader>
          <Form {...form}>
            <form
              onSubmit={form.handleSubmit((data) => void handleEdit(data))}
              className="space-y-4"
            >
              <FormField
                control={form.control}
                name="name"
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>Name</FormLabel>
                    <FormControl>
                      <Input
                        placeholder="Role name"
                        disabled={role.is_system}
                        {...field}
                      />
                    </FormControl>
                    <FormMessage />
                  </FormItem>
                )}
              />
              <FormField
                control={form.control}
                name="slug"
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>Slug</FormLabel>
                    <FormControl>
                      <Input
                        placeholder="role-slug"
                        disabled={role.is_system}
                        {...field}
                      />
                    </FormControl>
                    <FormMessage />
                  </FormItem>
                )}
              />
              <FormField
                control={form.control}
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
                control={form.control}
                name="permissions"
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>Permissions</FormLabel>
                    <FormControl>
                      <Input
                        placeholder="e.g. users:read, users:write"
                        {...field}
                      />
                    </FormControl>
                    <FormMessage />
                  </FormItem>
                )}
              />
              <FormField
                control={form.control}
                name="is_default"
                render={({ field }) => (
                  <FormItem className="flex items-center gap-2">
                    <FormControl>
                      <Checkbox
                        checked={field.value}
                        onCheckedChange={field.onChange}
                      />
                    </FormControl>
                    <FormLabel className="!mt-0">
                      Auto-assign to new users
                    </FormLabel>
                    <FormMessage />
                  </FormItem>
                )}
              />
              <DialogFooter>
                <Button
                  type="button"
                  variant="outline"
                  onClick={() => setEditOpen(false)}
                >
                  Cancel
                </Button>
                <Button type="submit" variant="primary" isLoading={updateMutation.isPending}>
                  Save Changes
                </Button>
              </DialogFooter>
            </form>
          </Form>
        </DialogContent>
      </Dialog>

      {/* Delete Confirmation */}
      <Dialog open={deleteOpen} onOpenChange={setDeleteOpen}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Delete Role</DialogTitle>
            <DialogDescription>
              Are you sure you want to delete &quot;{role.name}&quot;? It will
              be removed from all users and groups. This action cannot be
              undone.
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button variant="outline" onClick={() => setDeleteOpen(false)}>
              Cancel
            </Button>
            <Button
              variant="destructive"
              onClick={() => void handleDelete()}
              isLoading={deleteMutation.isPending}
            >
              Delete Role
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      {/* Bulk Assign Confirmation */}
      <Dialog open={bulkAssignOpen} onOpenChange={setBulkAssignOpen}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Assign Role to All Users</DialogTitle>
            <DialogDescription>
              This will assign the &quot;{role.name}&quot; role to every
              existing user who does not already have it. Users who already have
              this role will not be affected.
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button variant="outline" onClick={() => setBulkAssignOpen(false)}>
              Cancel
            </Button>
            <Button
              onClick={() => void handleBulkAssignAll()}
              isLoading={bulkAssignMutation.isPending}
            >
              Assign to All Users
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}
