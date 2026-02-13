import { useState } from "react";
import { useNavigate, useParams } from "@tanstack/react-router";
import { useForm } from "react-hook-form";
import { zodResolver } from "@hookform/resolvers/zod";
import {
  useAdminUser,
  useAdminUserSessions,
  useUpdateAdminUser,
  useSetUserRole,
  useSetUserStatus,
  useForcePasswordReset,
  useDeleteUser,
  useVerifyUserEmail,
  useRevokeUserSessions,
} from "@/hooks/use-admin";
import {
  useUserRoles,
  useUserGroups,
  useRoles,
  useAssignRole,
  useRevokeRole,
} from "@/hooks/use-rbac";
import { useAuthStore } from "@/stores/auth-store";
import { updateUserSchema, type UpdateUserFormData } from "@/schemas/admin";
import { formatDate, formatRelativeTime } from "@/lib/utils";
import { ApiError } from "@/lib/api-client";
import { PageHeader } from "@/components/shared/page-header";
import { DetailSection } from "@/components/shared/detail-section";
import { DetailRow } from "@/components/shared/detail-row";
import { Separator } from "@/components/ui/separator";
import { Skeleton } from "@/components/ui/skeleton";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
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
import { Input } from "@/components/ui/input";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import {
  Pencil,
  Trash2,
  ShieldCheck,
  ShieldOff,
  UserCheck,
  UserX,
  KeyRound,
  MailCheck,
  LogOut,
  AlertCircle,
} from "lucide-react";
import { toast } from "sonner";

type ConfirmAction =
  | "toggle-admin"
  | "toggle-status"
  | "delete"
  | "revoke-sessions"
  | "reset-password"
  | "verify-email"
  | null;

export function AdminUserDetailPage() {
  const { userId } = useParams({ strict: false }) as { userId: string };
  const navigate = useNavigate();
  const currentUser = useAuthStore((s) => s.user);

  const { data: user, isLoading, error } = useAdminUser(userId);
  const { data: sessionsData } = useAdminUserSessions(userId);

  const updateMutation = useUpdateAdminUser();
  const roleMutation = useSetUserRole();
  const statusMutation = useSetUserStatus();
  const passwordResetMutation = useForcePasswordReset();
  const deleteMutation = useDeleteUser();
  const verifyEmailMutation = useVerifyUserEmail();
  const revokeSessionsMutation = useRevokeUserSessions();

  const [editOpen, setEditOpen] = useState(false);
  const [confirmAction, setConfirmAction] = useState<ConfirmAction>(null);

  const isSelf = currentUser?.id === userId;
  const sessions = sessionsData?.sessions ?? [];

  const form = useForm<UpdateUserFormData>({
    resolver: zodResolver(updateUserSchema),
    defaultValues: {
      display_name: "",
      email: "",
      avatar_url: "",
    },
  });

  function openEditDialog() {
    if (!user) return;
    form.reset({
      display_name: user.display_name ?? "",
      email: user.email,
      avatar_url: user.avatar_url ?? "",
    });
    setEditOpen(true);
  }

  async function handleEdit(data: UpdateUserFormData) {
    const payload: Record<string, string> = {};
    if (data.display_name && data.display_name !== (user?.display_name ?? "")) {
      payload.display_name = data.display_name;
    }
    if (data.email && data.email !== user?.email) {
      payload.email = data.email;
    }
    if (data.avatar_url) {
      payload.avatar_url = data.avatar_url;
    }

    if (Object.keys(payload).length === 0) {
      setEditOpen(false);
      return;
    }

    try {
      await updateMutation.mutateAsync({ userId, data: payload });
      toast.success("User updated successfully");
      setEditOpen(false);
    } catch (err) {
      if (err instanceof ApiError) {
        form.setError("root", { message: err.message });
      } else {
        toast.error("Failed to update user");
      }
    }
  }

  async function handleToggleAdmin() {
    if (!user) return;
    try {
      await roleMutation.mutateAsync({
        userId,
        isAdmin: !user.is_admin,
      });
      toast.success(
        user.is_admin ? "Admin role revoked" : "Admin role granted",
      );
    } catch (err) {
      toast.error(
        err instanceof ApiError ? err.message : "Failed to update role",
      );
    } finally {
      setConfirmAction(null);
    }
  }

  async function handleToggleStatus() {
    if (!user) return;
    try {
      await statusMutation.mutateAsync({
        userId,
        isActive: !user.is_active,
      });
      toast.success(
        user.is_active
          ? "User disabled. Existing access tokens may remain valid for up to 15 minutes."
          : "User enabled",
      );
    } catch (err) {
      toast.error(
        err instanceof ApiError ? err.message : "Failed to update status",
      );
    } finally {
      setConfirmAction(null);
    }
  }

  async function handlePasswordReset() {
    try {
      await passwordResetMutation.mutateAsync(userId);
      toast.success("Password reset initiated");
    } catch (err) {
      toast.error(
        err instanceof ApiError ? err.message : "Failed to reset password",
      );
    } finally {
      setConfirmAction(null);
    }
  }

  async function handleDelete() {
    try {
      await deleteMutation.mutateAsync(userId);
      toast.success("User deleted");
      void navigate({ to: "/admin/users" });
    } catch (err) {
      toast.error(
        err instanceof ApiError ? err.message : "Failed to delete user",
      );
    } finally {
      setConfirmAction(null);
    }
  }

  async function handleVerifyEmail() {
    try {
      await verifyEmailMutation.mutateAsync(userId);
      toast.success("Email verified");
    } catch (err) {
      toast.error(
        err instanceof ApiError ? err.message : "Failed to verify email",
      );
    } finally {
      setConfirmAction(null);
    }
  }

  async function handleRevokeSessions() {
    try {
      const result = await revokeSessionsMutation.mutateAsync(userId);
      toast.success(
        `${String(result.revoked_count)} session(s) revoked. Existing access tokens may remain valid for up to 15 minutes.`,
      );
    } catch (err) {
      toast.error(
        err instanceof ApiError ? err.message : "Failed to revoke sessions",
      );
    } finally {
      setConfirmAction(null);
    }
  }

  if (isLoading) {
    return (
      <div className="space-y-6">
        <Skeleton className="h-10 w-64" />
        <Skeleton className="h-64 w-full" />
        <Skeleton className="h-48 w-full" />
      </div>
    );
  }

  if (error || !user) {
    return (
      <div className="flex flex-col items-center justify-center py-16 text-center">
        <AlertCircle className="mb-4 h-12 w-12 text-muted-foreground/50" />
        <h3 className="mb-2 font-display text-lg font-semibold">User not found</h3>
        <p className="mb-4 text-sm text-muted-foreground">
          The user you are looking for does not exist or has been deleted.
        </p>
        <Button
          variant="outline"
          onClick={() => void navigate({ to: "/admin/users" })}
        >
          Back to Users
        </Button>
      </div>
    );
  }

  return (
    <div className="space-y-6">
      <PageHeader
        breadcrumbs={[
          { label: "User Management", to: "/admin/users" },
          { label: user.email },
        ]}
        title={user.display_name ?? user.email}
        description={user.display_name ? user.email : undefined}
        actions={
          <>
            <Button variant="outline" size="sm" onClick={openEditDialog}>
              <Pencil className="mr-1 h-3 w-3" />
              Edit
            </Button>
            {!isSelf && (
              <Button
                variant="destructive"
                size="sm"
                onClick={() => setConfirmAction("delete")}
              >
                <Trash2 className="mr-1 h-3 w-3" />
                Delete
              </Button>
            )}
          </>
        }
      />

      <DetailSection title="User Information">
        <DetailRow label="ID" value={user.id} copyable mono />
        <DetailRow label="Email" value={user.email} copyable />
        <DetailRow
          label="Display Name"
          value={user.display_name ?? "Not set"}
        />
        <DetailRow
          label="Status"
          value={user.is_active ? "Active" : "Disabled"}
          badge
          badgeVariant={user.is_active ? "success" : "destructive"}
        />
        <DetailRow
          label="Role"
          value={user.is_admin ? "Admin" : "User"}
          badge
          badgeVariant={user.is_admin ? "default" : "secondary"}
        />
        <DetailRow
          label="Email Verified"
          value={user.email_verified ? "Verified" : "Unverified"}
          badge
          badgeVariant={user.email_verified ? "success" : "warning"}
        />
        <DetailRow
          label="MFA"
          value={user.mfa_enabled ? "Enabled" : "Disabled"}
          badge
          badgeVariant={user.mfa_enabled ? "success" : "secondary"}
        />
        <DetailRow label="Created" value={formatDate(user.created_at)} />
        <DetailRow
          label="Last Login"
          value={formatDate(user.last_login_at)}
        />
      </DetailSection>

      <Separator />

      <DetailSection title="Actions">
        <div className="flex flex-wrap gap-2">
          {!isSelf && (
            <>
              <Button
                variant="outline"
                size="sm"
                onClick={() => setConfirmAction("toggle-admin")}
              >
                {user.is_admin ? (
                  <ShieldOff className="mr-1 h-3 w-3" />
                ) : (
                  <ShieldCheck className="mr-1 h-3 w-3" />
                )}
                {user.is_admin ? "Change to User" : "Change to Admin"}
              </Button>
              <Button
                variant="outline"
                size="sm"
                onClick={() => setConfirmAction("toggle-status")}
              >
                {user.is_active ? (
                  <UserX className="mr-1 h-3 w-3" />
                ) : (
                  <UserCheck className="mr-1 h-3 w-3" />
                )}
                {user.is_active ? "Disable User" : "Enable User"}
              </Button>
            </>
          )}
          {!user.email_verified && (
            <Button
              variant="outline"
              size="sm"
              onClick={() => setConfirmAction("verify-email")}
            >
              <MailCheck className="mr-1 h-3 w-3" />
              Verify Email
            </Button>
          )}
          <Button
            variant="outline"
            size="sm"
            onClick={() => setConfirmAction("reset-password")}
          >
            <KeyRound className="mr-1 h-3 w-3" />
            Reset Password
          </Button>
          <Button
            variant="outline"
            size="sm"
            onClick={() => setConfirmAction("revoke-sessions")}
          >
            <LogOut className="mr-1 h-3 w-3" />
            Revoke Sessions
          </Button>
        </div>
      </DetailSection>

      <Separator />

      <UserRolesSection userId={userId} />

      <Separator />

      <UserGroupsSection userId={userId} />

      <Separator />

      <DetailSection title="Sessions">
        {sessions.length === 0 ? (
          <p className="text-sm text-muted-foreground">No sessions found.</p>
        ) : (
          <div className="rounded-xl border border-border">
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>IP Address</TableHead>
                  <TableHead>User Agent</TableHead>
                  <TableHead>Created</TableHead>
                  <TableHead>Expires</TableHead>
                  <TableHead>Last Active</TableHead>
                  <TableHead>Status</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {sessions.map((session) => (
                  <TableRow key={session.id}>
                    <TableCell className="font-mono text-xs">
                      {session.ip_address ?? "--"}
                    </TableCell>
                    <TableCell
                      className="max-w-[200px] truncate text-xs"
                      title={session.user_agent ?? undefined}
                    >
                      {session.user_agent ?? "--"}
                    </TableCell>
                    <TableCell className="text-muted-foreground text-xs">
                      {formatRelativeTime(session.created_at)}
                    </TableCell>
                    <TableCell className="text-muted-foreground text-xs">
                      {formatDate(session.expires_at)}
                    </TableCell>
                    <TableCell className="text-muted-foreground text-xs">
                      {formatRelativeTime(session.last_active_at)}
                    </TableCell>
                    <TableCell>
                      <Badge
                        variant={session.revoked ? "destructive" : "success"}
                      >
                        {session.revoked ? "Revoked" : "Active"}
                      </Badge>
                    </TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          </div>
        )}
      </DetailSection>

      {/* Edit Dialog */}
      <Dialog open={editOpen} onOpenChange={setEditOpen}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Edit User</DialogTitle>
            <DialogDescription>
              Update user profile fields for {user.email}.
            </DialogDescription>
          </DialogHeader>
          <Form {...form}>
            <form
              onSubmit={form.handleSubmit((data) => void handleEdit(data))}
              className="space-y-4"
            >
              {form.formState.errors.root && (
                <div className="rounded-md bg-destructive/10 p-3 text-sm text-destructive">
                  {form.formState.errors.root.message}
                </div>
              )}
              <FormField
                control={form.control}
                name="display_name"
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>Display Name</FormLabel>
                    <FormControl>
                      <Input placeholder="Display name" {...field} />
                    </FormControl>
                    <FormMessage />
                  </FormItem>
                )}
              />
              <FormField
                control={form.control}
                name="email"
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>Email</FormLabel>
                    <FormControl>
                      <Input placeholder="user@example.com" {...field} />
                    </FormControl>
                    <FormMessage />
                  </FormItem>
                )}
              />
              <FormField
                control={form.control}
                name="avatar_url"
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>Avatar URL</FormLabel>
                    <FormControl>
                      <Input placeholder="https://..." {...field} />
                    </FormControl>
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
                <Button type="submit" isLoading={updateMutation.isPending}>
                  Save Changes
                </Button>
              </DialogFooter>
            </form>
          </Form>
        </DialogContent>
      </Dialog>

      {/* Confirmation Dialogs */}
      <ConfirmDialog
        open={confirmAction === "toggle-admin"}
        onOpenChange={(open) => {
          if (!open) setConfirmAction(null);
        }}
        title={user.is_admin ? "Change Role to User" : "Change Role to Admin"}
        description={
          user.is_admin
            ? `Are you sure you want to change ${user.email} from Admin to User? They will lose all admin privileges.`
            : `Are you sure you want to change ${user.email} from User to Admin? They will gain full admin privileges.`
        }
        confirmLabel={user.is_admin ? "Change to User" : "Change to Admin"}
        variant={user.is_admin ? "destructive" : "default"}
        isPending={roleMutation.isPending}
        onConfirm={() => void handleToggleAdmin()}
      />

      <ConfirmDialog
        open={confirmAction === "toggle-status"}
        onOpenChange={(open) => {
          if (!open) setConfirmAction(null);
        }}
        title={user.is_active ? "Disable User" : "Enable User"}
        description={
          user.is_active
            ? `Are you sure you want to disable ${user.email}? They will be immediately logged out.`
            : `Are you sure you want to enable ${user.email}?`
        }
        confirmLabel={user.is_active ? "Disable" : "Enable"}
        variant={user.is_active ? "destructive" : "default"}
        isPending={statusMutation.isPending}
        onConfirm={() => void handleToggleStatus()}
      />

      <ConfirmDialog
        open={confirmAction === "delete"}
        onOpenChange={(open) => {
          if (!open) setConfirmAction(null);
        }}
        title="Delete User"
        description={`Are you sure you want to permanently delete ${user.email}? This cannot be undone. All sessions, tokens, connections, and API keys will be deleted.`}
        confirmLabel="Delete User"
        variant="destructive"
        isPending={deleteMutation.isPending}
        onConfirm={() => void handleDelete()}
      />

      <ConfirmDialog
        open={confirmAction === "revoke-sessions"}
        onOpenChange={(open) => {
          if (!open) setConfirmAction(null);
        }}
        title="Revoke All Sessions"
        description={`Are you sure you want to revoke all sessions for ${user.email}? They will be immediately logged out.`}
        confirmLabel="Revoke Sessions"
        variant="destructive"
        isPending={revokeSessionsMutation.isPending}
        onConfirm={() => void handleRevokeSessions()}
      />

      <ConfirmDialog
        open={confirmAction === "reset-password"}
        onOpenChange={(open) => {
          if (!open) setConfirmAction(null);
        }}
        title="Force Password Reset"
        description={`Send a password reset email to ${user.email}? Their current sessions will be revoked.`}
        confirmLabel="Reset Password"
        variant="default"
        isPending={passwordResetMutation.isPending}
        onConfirm={() => void handlePasswordReset()}
      />

      <ConfirmDialog
        open={confirmAction === "verify-email"}
        onOpenChange={(open) => {
          if (!open) setConfirmAction(null);
        }}
        title="Verify Email"
        description={`Manually verify the email address for ${user.email}?`}
        confirmLabel="Verify Email"
        variant="default"
        isPending={verifyEmailMutation.isPending}
        onConfirm={() => void handleVerifyEmail()}
      />
    </div>
  );
}

function UserRolesSection({ userId }: { readonly userId: string }) {
  const { data: userRolesData, isLoading } = useUserRoles(userId);
  const { data: allRolesData } = useRoles();
  const assignMutation = useAssignRole();
  const revokeMutation = useRevokeRole();
  const [assignOpen, setAssignOpen] = useState(false);
  const [selectedRoleId, setSelectedRoleId] = useState("");

  const directRoles = userRolesData?.direct_roles ?? [];
  const inheritedRoles = userRolesData?.inherited_roles ?? [];
  const effectivePermissions = userRolesData?.effective_permissions ?? [];
  const allRoles = allRolesData?.roles ?? [];

  const assignedRoleIds = new Set(directRoles.map((r) => r.id));
  const availableRoles = allRoles.filter((r) => !assignedRoleIds.has(r.id));

  async function handleAssign() {
    if (!selectedRoleId) return;
    try {
      await assignMutation.mutateAsync({ userId, roleId: selectedRoleId });
      toast.success("Role assigned");
      setSelectedRoleId("");
      setAssignOpen(false);
    } catch (err) {
      toast.error(
        err instanceof ApiError ? err.message : "Failed to assign role",
      );
    }
  }

  async function handleRevoke(roleId: string) {
    try {
      await revokeMutation.mutateAsync({ userId, roleId });
      toast.success("Role revoked");
    } catch (err) {
      toast.error(
        err instanceof ApiError ? err.message : "Failed to revoke role",
      );
    }
  }

  if (isLoading) {
    return (
      <DetailSection title="Roles">
        <Skeleton className="h-20 w-full" />
      </DetailSection>
    );
  }

  return (
    <DetailSection title="Roles">
      <div className="mb-3">
        <Button
          size="sm"
          variant="outline"
          onClick={() => setAssignOpen(true)}
        >
          Assign Role
        </Button>
      </div>

      {directRoles.length > 0 && (
        <div className="mb-3">
          <p className="mb-1 text-xs font-medium text-muted-foreground">
            Direct Roles
          </p>
          <div className="flex flex-wrap gap-2">
            {directRoles.map((role) => (
              <Badge
                key={role.id}
                variant="default"
                className="gap-1"
              >
                {role.name}
                {!role.is_system && (
                  <button
                    type="button"
                    className="ml-1 rounded-full hover:bg-primary-foreground/20 disabled:opacity-50"
                    onClick={() => void handleRevoke(role.id)}
                    disabled={revokeMutation.isPending}
                    aria-label={`Revoke ${role.name}`}
                  >
                    x
                  </button>
                )}
              </Badge>
            ))}
          </div>
        </div>
      )}

      {inheritedRoles.length > 0 && (
        <div className="mb-3">
          <p className="mb-1 text-xs font-medium text-muted-foreground">
            Inherited from Groups
          </p>
          <div className="flex flex-wrap gap-2">
            {inheritedRoles.map((role) => (
              <Badge key={role.id} variant="secondary">
                {role.name}
              </Badge>
            ))}
          </div>
        </div>
      )}

      {effectivePermissions.length > 0 && (
        <div>
          <p className="mb-1 text-xs font-medium text-muted-foreground">
            Effective Permissions
          </p>
          <div className="flex flex-wrap gap-1">
            {effectivePermissions.map((perm) => (
              <Badge
                key={perm}
                variant="outline"
                className="font-mono text-xs"
              >
                {perm}
              </Badge>
            ))}
          </div>
        </div>
      )}

      {directRoles.length === 0 && inheritedRoles.length === 0 && (
        <p className="text-sm text-muted-foreground">No roles assigned.</p>
      )}

      <Dialog open={assignOpen} onOpenChange={setAssignOpen}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Assign Role</DialogTitle>
            <DialogDescription>
              Select a role to assign to this user.
            </DialogDescription>
          </DialogHeader>
          <div className="space-y-4">
            <Select value={selectedRoleId} onValueChange={setSelectedRoleId}>
              <SelectTrigger>
                <SelectValue placeholder="Select a role..." />
              </SelectTrigger>
              <SelectContent>
                {availableRoles.map((role) => (
                  <SelectItem key={role.id} value={role.id}>
                    {role.name} ({role.slug})
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
          <DialogFooter>
            <Button variant="outline" onClick={() => setAssignOpen(false)}>
              Cancel
            </Button>
            <Button
              onClick={() => void handleAssign()}
              disabled={!selectedRoleId}
              isLoading={assignMutation.isPending}
            >
              Assign Role
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </DetailSection>
  );
}

function UserGroupsSection({ userId }: { readonly userId: string }) {
  const { data: userGroupsData, isLoading } = useUserGroups(userId);
  const groups = userGroupsData?.groups ?? [];

  if (isLoading) {
    return (
      <DetailSection title="Groups">
        <Skeleton className="h-20 w-full" />
      </DetailSection>
    );
  }

  return (
    <DetailSection title="Groups">
      {groups.length === 0 ? (
        <p className="text-sm text-muted-foreground">No group memberships.</p>
      ) : (
        <div className="flex flex-wrap gap-2">
          {groups.map((group) => (
            <Badge key={group.id} variant="outline">
              {group.name}
              {group.roles.length > 0 && (
                <span className="ml-1 text-muted-foreground">
                  ({group.roles.map((r) => r.name).join(", ")})
                </span>
              )}
            </Badge>
          ))}
        </div>
      )}
    </DetailSection>
  );
}

interface ConfirmDialogProps {
  readonly open: boolean;
  readonly onOpenChange: (open: boolean) => void;
  readonly title: string;
  readonly description: string;
  readonly confirmLabel: string;
  readonly variant: "default" | "destructive";
  readonly isPending: boolean;
  readonly onConfirm: () => void;
}

function ConfirmDialog({
  open,
  onOpenChange,
  title,
  description,
  confirmLabel,
  variant,
  isPending,
  onConfirm,
}: ConfirmDialogProps) {
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>{title}</DialogTitle>
          <DialogDescription>{description}</DialogDescription>
        </DialogHeader>
        <DialogFooter>
          <Button variant="outline" onClick={() => onOpenChange(false)}>
            Cancel
          </Button>
          <Button
            variant={variant}
            onClick={onConfirm}
            isLoading={isPending}
          >
            {confirmLabel}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
