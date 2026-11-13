import { useUiTranslation } from "@/i18n/use-ui-translation";
import { translateUi } from "@/i18n/use-ui-translation";
import { translateErrorMessage } from "@/lib/error-messages";
import { useState } from "react";
import { useNavigate } from "@tanstack/react-router";
import { zodResolver } from "@hookform/resolvers/zod";
import { useRoles, useCreateRole, useDeleteRole } from "@/hooks/use-rbac";
import { createRoleSchema, type CreateRoleFormData } from "@/schemas/rbac";
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
import { SmartRemoteIcon } from "@/components/icons/empty-state";
import { toast } from "sonner";
import { Checkbox } from "@/components/ui/checkbox";

export function AdminRolesPage() {
  useUiTranslation();
  const navigate = useNavigate();
  const currentUser = useAuthStore((s) => s.user);
  const canWrite = canAdminWrite(currentUser);
  const [createOpen, setCreateOpen] = useState(false);
  const [deleteRoleId, setDeleteRoleId] = useState<string | null>(null);

  const { data, isLoading, error } = useRoles();
  const createMutation = useCreateRole();
  const deleteMutation = useDeleteRole();

  const roles = data?.roles ?? [];

  const createForm = useAppForm<CreateRoleFormData>({
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
      toast.success(translateUi("Role created successfully"));
      setCreateOpen(false);
    } catch (err) {
      toast.error(
        translateErrorMessage(err instanceof ApiError ? err.message : "Failed to create role"),
      );
    }
  }

  async function handleDelete() {
    if (!deleteRoleId) return;
    try {
      await deleteMutation.mutateAsync(deleteRoleId);
      toast.success(translateUi("Role deleted"));
    } catch (err) {
      toast.error(
        translateErrorMessage(err instanceof ApiError ? err.message : "Failed to delete role"),
      );
    } finally {
      setDeleteRoleId(null);
    }
  }

  return (
    <div className="space-y-8">
      <PageHeader
        title={translateUi("Role Management")}
        description={translateUi("Manage roles and permissions for your organization.")}
        actions={
          canWrite ? (
            <AddCtaButton label={translateUi("Add Role")} onClick={openCreateDialog} />
          ) : null
        }
      />

      {isLoading ? (
        <div className="space-y-2">
          {Array.from({ length: 5 }).map((_, i) => (
            <Skeleton key={`role-skel-${String(i)}`} className="h-12 w-full" />
          ))}
        </div>
      ) : error ? (
        <div className="flex flex-col items-center justify-center gap-1 py-12 text-center">
          <SmartRemoteIcon className="h-64 w-64 text-muted-foreground" />
          <div className="space-y-1">
            <p className="type-row-title text-muted-foreground">{translateUi("Failed to load roles")}</p>
            <p className="text-13 text-muted-foreground">{translateUi("Please try again later.")}</p>
          </div>
        </div>
      ) : roles.length === 0 ? (
        <div className="flex flex-col items-center justify-center gap-1 py-12 text-center">
          <SmartRemoteIcon className="h-64 w-64 text-muted-foreground" />
          <div className="space-y-1">
            <p className="type-row-title text-muted-foreground">{translateUi("No roles found")}</p>
            <p className="text-13 text-muted-foreground">{translateUi("There are no roles to display.")}</p>
          </div>
        </div>
      ) : (
        <>
        {/* Mobile cards */}
        <div className="flex flex-col gap-3 md:hidden">
          {roles.map((r) => (
            <div
              key={r.id}
              className="rounded-xl border border-border/50 bg-card p-4 transition-colors hover:bg-overlay cursor-pointer"
              tabIndex={0}
              role="link"
              onClick={() =>
                void navigate({
                  to: "/admin/roles/$roleId",
                  params: { roleId: r.id },
                })
              }
              onKeyDown={(e) => {
                if (e.key === "Enter" || e.key === " ") {
                  e.preventDefault();
                  void navigate({
                    to: "/admin/roles/$roleId",
                    params: { roleId: r.id },
                  });
                }
              }}
            >
              <div className="flex items-start justify-between gap-2">
                <div className="min-w-0 flex-1">
                  <p className="type-row-title truncate">{r.name}</p>
                  <p className="text-13 text-muted-foreground font-mono truncate mt-0.5">
                    {r.slug}
                  </p>
                </div>
                <div className="flex items-center gap-1.5 shrink-0">
                  <Badge variant="secondary">
                    {r.is_system ? translateUi("System") : translateUi("Custom")}
                  </Badge>
                </div>
              </div>
              <div className="mt-3 flex items-center gap-3 text-13 text-muted-foreground">
                <span>
                  {String(r.permissions.length)}{translateUi(" permission")}{r.permissions.length !== 1 ? "s" : ""}
                </span>
                <span className="text-border">|</span>
                <span>{translateUi("Created ")}{formatDate(r.created_at)}</span>
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
                <TableHead>{translateUi("Permissions")}</TableHead>
                <TableHead>{translateUi("Type")}</TableHead>
                <TableHead>{translateUi("Default")}</TableHead>
                <TableHead>{translateUi("Created")}</TableHead>
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
                  <TableCell className="font-normal">{role.name}</TableCell>
                  <TableCell className="font-mono text-14">
                    {role.slug}
                  </TableCell>
                  <TableCell>
                    <span className="text-muted-foreground text-13">
                      {String(role.permissions.length)}{translateUi(" permission")}{role.permissions.length !== 1 ? "s" : ""}
                    </span>
                  </TableCell>
                  <TableCell>
                    {role.is_system ? (
                      <Badge variant="secondary">{translateUi("System")}</Badge>
                    ) : (
                      <Badge variant="secondary">{translateUi("Custom")}</Badge>
                    )}
                  </TableCell>
                  <TableCell>
                    {role.is_default ? (
                      <Badge variant="success">{translateUi("Yes")}</Badge>
                    ) : (
                      <span className="text-muted-foreground text-13">{translateUi("No")}</span>
                    )}
                  </TableCell>
                  <TableCell className="text-muted-foreground">
                    {formatDate(role.created_at)}
                  </TableCell>
                  <TableCell>
                    {canWrite && !role.is_system && (
                      <Button
                        variant="ghost"
                        size="icon"
                        className="h-8 w-8"
                        onClick={(e) => {
                          e.stopPropagation();
                          setDeleteRoleId(role.id);
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

      {/* Create Role Dialog */}
      <Dialog open={createOpen} onOpenChange={setCreateOpen}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>{translateUi("Create Role")}</DialogTitle>
            <DialogDescription>{translateUi("Create a new role with specific permissions.")}</DialogDescription>
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
                      <Input placeholder={translateUi("e.g. Editor")} {...field} />
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
                      <Input placeholder={translateUi("e.g. editor")} {...field} />
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
                name="permissions"
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>{translateUi("Permissions")}</FormLabel>
                    <FormControl>
                      <Input
                        placeholder={translateUi("e.g. users:read, users:write")}
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
                  <FormItem className="flex items-center gap-2 space-y-0">
                    <FormControl>
                      <Checkbox
                        checked={field.value}
                        onCheckedChange={field.onChange}
                      />
                    </FormControl>
                    <FormLabel>{translateUi("Auto-assign to new users")}</FormLabel>
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
                <Button type="submit" variant="primary" isLoading={createMutation.isPending}>{translateUi("Create Role")}</Button>
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
            <DialogTitle>{translateUi("Delete Role")}</DialogTitle>
            <DialogDescription>{translateUi("Are you sure you want to delete this role? It will be removed from all users and groups. This action cannot be undone.")}</DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button variant="outline" onClick={() => setDeleteRoleId(null)}>{translateUi("Cancel")}</Button>
            <Button
              variant="destructive"
              onClick={() => void handleDelete()}
              isLoading={deleteMutation.isPending}
            >{translateUi("Delete Role")}</Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}
