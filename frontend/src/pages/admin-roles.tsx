import { useState } from "react";
import { useNavigate } from "@tanstack/react-router";
import { useForm } from "react-hook-form";
import { zodResolver } from "@hookform/resolvers/zod";
import { useRoles, useCreateRole, useDeleteRole } from "@/hooks/use-rbac";
import { createRoleSchema, type CreateRoleFormData } from "@/schemas/rbac";
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
import { ShieldCheck, Plus, Trash2 } from "lucide-react";
import { toast } from "sonner";
import { Checkbox } from "@/components/ui/checkbox";

export function AdminRolesPage() {
  const navigate = useNavigate();
  const [createOpen, setCreateOpen] = useState(false);
  const [deleteRoleId, setDeleteRoleId] = useState<string | null>(null);

  const { data, isLoading, error } = useRoles();
  const createMutation = useCreateRole();
  const deleteMutation = useDeleteRole();

  const roles = data?.roles ?? [];

  const createForm = useForm<CreateRoleFormData>({
    resolver: zodResolver(createRoleSchema),
    defaultValues: {
      name: "",
      slug: "",
      description: "",
      permissions: "",
      is_default: false,
      client_id: "",
    },
  });

  function openCreateDialog() {
    createForm.reset({
      name: "",
      slug: "",
      description: "",
      permissions: "",
      is_default: false,
      client_id: "",
    });
    setCreateOpen(true);
  }

  async function handleCreate(data: CreateRoleFormData) {
    try {
      const permissions = data.permissions
        ? data.permissions
            .split(",")
            .map((p) => p.trim())
            .filter((p) => p.length > 0)
        : [];
      await createMutation.mutateAsync({
        name: data.name,
        slug: data.slug,
        description: data.description || undefined,
        permissions,
        is_default: data.is_default,
        client_id: data.client_id || undefined,
      });
      toast.success("Role created successfully");
      setCreateOpen(false);
    } catch (err) {
      if (err instanceof ApiError) {
        createForm.setError("root", { message: err.message });
      } else {
        toast.error("Failed to create role");
      }
    }
  }

  async function handleDelete() {
    if (!deleteRoleId) return;
    try {
      await deleteMutation.mutateAsync(deleteRoleId);
      toast.success("Role deleted");
    } catch (err) {
      toast.error(
        err instanceof ApiError ? err.message : "Failed to delete role",
      );
    } finally {
      setDeleteRoleId(null);
    }
  }

  return (
    <div className="space-y-8">
      <PageHeader
        title="Role Management"
        description="Manage roles and permissions for your organization."
        actions={
          <Button size="sm" onClick={openCreateDialog}>
            <Plus className="mr-1 h-4 w-4" />
            Add Role
          </Button>
        }
      />

      {isLoading ? (
        <div className="space-y-2">
          {Array.from({ length: 5 }).map((_, i) => (
            <Skeleton key={`role-skel-${String(i)}`} className="h-12 w-full" />
          ))}
        </div>
      ) : error ? (
        <div className="flex flex-col items-center justify-center py-12 text-center">
          <ShieldCheck className="mb-4 h-12 w-12 text-muted-foreground/50" />
          <p className="text-sm text-muted-foreground">
            Failed to load roles. Please try again.
          </p>
        </div>
      ) : roles.length === 0 ? (
        <div className="flex flex-col items-center justify-center py-12 text-center">
          <ShieldCheck className="mb-4 h-12 w-12 text-muted-foreground/50" />
          <p className="text-sm text-muted-foreground">No roles found.</p>
        </div>
      ) : (
        <div className="rounded-xl border border-border">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>Name</TableHead>
                <TableHead>Slug</TableHead>
                <TableHead>Permissions</TableHead>
                <TableHead>Type</TableHead>
                <TableHead>Default</TableHead>
                <TableHead>Created</TableHead>
                <TableHead className="w-[60px]" />
              </TableRow>
            </TableHeader>
            <TableBody>
              {roles.map((role) => (
                <TableRow
                  key={role.id}
                  className="cursor-pointer"
                  tabIndex={0}
                  role="link"
                  onClick={() =>
                    void navigate({
                      to: "/admin/roles/$roleId",
                      params: { roleId: role.id },
                    })
                  }
                  onKeyDown={(e) => {
                    if (e.key === "Enter" || e.key === " ") {
                      e.preventDefault();
                      void navigate({
                        to: "/admin/roles/$roleId",
                        params: { roleId: role.id },
                      });
                    }
                  }}
                >
                  <TableCell className="font-medium">{role.name}</TableCell>
                  <TableCell className="font-mono text-xs">
                    {role.slug}
                  </TableCell>
                  <TableCell>
                    <span className="text-muted-foreground text-xs">
                      {String(role.permissions.length)} permission
                      {role.permissions.length !== 1 ? "s" : ""}
                    </span>
                  </TableCell>
                  <TableCell>
                    {role.is_system ? (
                      <Badge variant="secondary">System</Badge>
                    ) : (
                      <Badge variant="outline">Custom</Badge>
                    )}
                  </TableCell>
                  <TableCell>
                    {role.is_default ? (
                      <Badge variant="success">Yes</Badge>
                    ) : (
                      <span className="text-muted-foreground text-xs">No</span>
                    )}
                  </TableCell>
                  <TableCell className="text-muted-foreground">
                    {formatDate(role.created_at)}
                  </TableCell>
                  <TableCell>
                    {!role.is_system && (
                      <Button
                        variant="ghost"
                        size="icon"
                        className="h-8 w-8"
                        onClick={(e) => {
                          e.stopPropagation();
                          setDeleteRoleId(role.id);
                        }}
                      >
                        <Trash2 className="h-4 w-4 text-muted-foreground" />
                      </Button>
                    )}
                  </TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        </div>
      )}

      {/* Create Role Dialog */}
      <Dialog open={createOpen} onOpenChange={setCreateOpen}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Create Role</DialogTitle>
            <DialogDescription>
              Create a new role with specific permissions.
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
                      <Input placeholder="e.g. Editor" {...field} />
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
                      <Input placeholder="e.g. editor" {...field} />
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
                control={createForm.control}
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
                  onClick={() => setCreateOpen(false)}
                >
                  Cancel
                </Button>
                <Button type="submit" isLoading={createMutation.isPending}>
                  Create Role
                </Button>
              </DialogFooter>
            </form>
          </Form>
        </DialogContent>
      </Dialog>

      {/* Delete Confirmation Dialog */}
      <Dialog
        open={deleteRoleId !== null}
        onOpenChange={(open) => {
          if (!open) setDeleteRoleId(null);
        }}
      >
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Delete Role</DialogTitle>
            <DialogDescription>
              Are you sure you want to delete this role? It will be removed from
              all users and groups. This action cannot be undone.
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button variant="outline" onClick={() => setDeleteRoleId(null)}>
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
    </div>
  );
}
