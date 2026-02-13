import { useState } from "react";
import { useNavigate, useParams } from "@tanstack/react-router";
import { useForm } from "react-hook-form";
import { zodResolver } from "@hookform/resolvers/zod";
import {
  useGroup,
  useUpdateGroup,
  useDeleteGroup,
  useGroupMembers,
  useAddGroupMember,
  useRemoveGroupMember,
  useRoles,
} from "@/hooks/use-rbac";
import { updateGroupSchema, type UpdateGroupFormData } from "@/schemas/rbac";
import { ApiError } from "@/lib/api-client";
import { formatDate } from "@/lib/utils";
import { PageHeader } from "@/components/shared/page-header";
import { DetailSection } from "@/components/shared/detail-section";
import { DetailRow } from "@/components/shared/detail-row";
import { Separator } from "@/components/ui/separator";
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
import { Pencil, Trash2, AlertCircle, UserPlus, UserMinus } from "lucide-react";
import { toast } from "sonner";

export function AdminGroupDetailPage() {
  const { groupId } = useParams({ strict: false }) as { groupId: string };
  const navigate = useNavigate();

  const { data: group, isLoading, error } = useGroup(groupId);
  const { data: membersData } = useGroupMembers(groupId);
  const { data: rolesData } = useRoles();
  const updateMutation = useUpdateGroup();
  const deleteMutation = useDeleteGroup();
  const addMemberMutation = useAddGroupMember();
  const removeMemberMutation = useRemoveGroupMember();

  const [editOpen, setEditOpen] = useState(false);
  const [deleteOpen, setDeleteOpen] = useState(false);
  const [addMemberOpen, setAddMemberOpen] = useState(false);
  const [newMemberUserId, setNewMemberUserId] = useState("");
  const [removeMemberId, setRemoveMemberId] = useState<string | null>(null);

  const members = membersData?.members ?? [];
  const availableRoles = rolesData?.roles ?? [];

  const form = useForm<UpdateGroupFormData>({
    resolver: zodResolver(updateGroupSchema),
    defaultValues: {
      name: "",
      slug: "",
      description: "",
      role_ids: "",
      parent_group_id: "",
    },
  });

  function openEditDialog() {
    if (!group) return;
    form.reset({
      name: group.name,
      slug: group.slug,
      description: group.description ?? "",
      role_ids: group.roles.map((r) => r.id).join(","),
      parent_group_id: group.parent_group_id ?? "",
    });
    setEditOpen(true);
  }

  async function handleEdit(data: UpdateGroupFormData) {
    try {
      const roleIds = data.role_ids
        ? data.role_ids
            .split(",")
            .map((id) => id.trim())
            .filter((id) => id.length > 0)
        : [];
      await updateMutation.mutateAsync({
        groupId,
        data: {
          name: data.name,
          slug: data.slug,
          description: data.description || undefined,
          role_ids: roleIds,
          parent_group_id: data.parent_group_id || undefined,
        },
      });
      toast.success("Group updated successfully");
      setEditOpen(false);
    } catch (err) {
      if (err instanceof ApiError) {
        form.setError("root", { message: err.message });
      } else {
        toast.error("Failed to update group");
      }
    }
  }

  async function handleDelete() {
    try {
      await deleteMutation.mutateAsync(groupId);
      toast.success("Group deleted");
      void navigate({ to: "/admin/groups" });
    } catch (err) {
      toast.error(
        err instanceof ApiError ? err.message : "Failed to delete group",
      );
    } finally {
      setDeleteOpen(false);
    }
  }

  async function handleAddMember() {
    if (!newMemberUserId.trim()) return;
    try {
      await addMemberMutation.mutateAsync({
        groupId,
        userId: newMemberUserId.trim(),
      });
      toast.success("Member added");
      setNewMemberUserId("");
      setAddMemberOpen(false);
    } catch (err) {
      toast.error(
        err instanceof ApiError ? err.message : "Failed to add member",
      );
    }
  }

  async function handleRemoveMember() {
    if (!removeMemberId) return;
    try {
      await removeMemberMutation.mutateAsync({
        groupId,
        userId: removeMemberId,
      });
      toast.success("Member removed");
    } catch (err) {
      toast.error(
        err instanceof ApiError ? err.message : "Failed to remove member",
      );
    } finally {
      setRemoveMemberId(null);
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

  if (error || !group) {
    return (
      <div className="flex flex-col items-center justify-center py-16 text-center">
        <AlertCircle className="mb-4 h-12 w-12 text-muted-foreground/50" />
        <h3 className="mb-2 font-display text-lg font-semibold">Group not found</h3>
        <p className="mb-4 text-sm text-muted-foreground">
          The group you are looking for does not exist or has been deleted.
        </p>
        <Button
          variant="outline"
          onClick={() => void navigate({ to: "/admin/groups" })}
        >
          Back to Groups
        </Button>
      </div>
    );
  }

  return (
    <div className="space-y-6">
      <PageHeader
        breadcrumbs={[
          { label: "Group Management", to: "/admin/groups" },
          { label: group.name },
        ]}
        title={group.name}
        description={group.description ?? undefined}
        actions={
          <>
            <Button variant="outline" size="sm" onClick={openEditDialog}>
              <Pencil className="mr-1 h-3 w-3" />
              Edit
            </Button>
            <Button
              variant="destructive"
              size="sm"
              onClick={() => setDeleteOpen(true)}
            >
              <Trash2 className="mr-1 h-3 w-3" />
              Delete
            </Button>
          </>
        }
      />

      <DetailSection title="Group Information">
        <DetailRow label="ID" value={group.id} copyable mono />
        <DetailRow label="Name" value={group.name} />
        <DetailRow label="Slug" value={group.slug} copyable mono />
        <DetailRow
          label="Members"
          value={String(group.member_count)}
        />
        {group.parent_group_id && (
          <DetailRow
            label="Parent Group"
            value={group.parent_group_id}
            copyable
            mono
          />
        )}
        <DetailRow label="Created" value={formatDate(group.created_at)} />
        <DetailRow label="Updated" value={formatDate(group.updated_at)} />
      </DetailSection>

      <Separator />

      <DetailSection title="Inherited Roles">
        {group.roles.length === 0 ? (
          <p className="text-sm text-muted-foreground">
            No roles assigned to this group.
          </p>
        ) : (
          <div className="flex flex-wrap gap-2">
            {group.roles.map((role) => (
              <Badge key={role.id} variant="outline">
                {role.name}
              </Badge>
            ))}
          </div>
        )}
      </DetailSection>

      <Separator />

      <DetailSection title="Members">
        <div className="mb-3">
          <Button
            size="sm"
            variant="outline"
            onClick={() => setAddMemberOpen(true)}
          >
            <UserPlus className="mr-1 h-4 w-4" />
            Add Member
          </Button>
        </div>
        {members.length === 0 ? (
          <p className="text-sm text-muted-foreground">
            No members in this group.
          </p>
        ) : (
          <div className="rounded-xl border border-border">
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>Email</TableHead>
                  <TableHead>Name</TableHead>
                  <TableHead className="w-[60px]" />
                </TableRow>
              </TableHeader>
              <TableBody>
                {members.map((member) => (
                  <TableRow key={member.id}>
                    <TableCell className="font-medium">
                      {member.email}
                    </TableCell>
                    <TableCell>
                      {member.display_name ?? (
                        <span className="text-muted-foreground">--</span>
                      )}
                    </TableCell>
                    <TableCell>
                      <Button
                        variant="ghost"
                        size="icon"
                        className="h-8 w-8"
                        onClick={() => setRemoveMemberId(member.id)}
                      >
                        <UserMinus className="h-4 w-4 text-muted-foreground" />
                      </Button>
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
            <DialogTitle>Edit Group</DialogTitle>
            <DialogDescription>
              Update group details for {group.name}.
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
                name="name"
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>Name</FormLabel>
                    <FormControl>
                      <Input placeholder="Group name" {...field} />
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
                      <Input placeholder="group-slug" {...field} />
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
                name="role_ids"
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>Roles</FormLabel>
                    <FormControl>
                      <select
                        className="flex w-full rounded-[10px] border border-input bg-popover px-3 py-2 text-[13px] text-foreground shadow-sm transition-colors focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring [&_option]:bg-popover [&_option]:text-foreground [&_option:checked]:bg-primary/20"
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

      {/* Delete Confirmation */}
      <Dialog open={deleteOpen} onOpenChange={setDeleteOpen}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Delete Group</DialogTitle>
            <DialogDescription>
              Are you sure you want to delete &quot;{group.name}&quot;? All
              members will be removed. This action cannot be undone.
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
              Delete Group
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      {/* Add Member Dialog */}
      <Dialog open={addMemberOpen} onOpenChange={setAddMemberOpen}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Add Member</DialogTitle>
            <DialogDescription>
              Enter the user ID to add to this group.
            </DialogDescription>
          </DialogHeader>
          <div className="space-y-4">
            <div className="space-y-2">
              <label className="text-sm font-medium" htmlFor="add-member-id">
                User ID
              </label>
              <Input
                id="add-member-id"
                placeholder="Enter user ID"
                value={newMemberUserId}
                onChange={(e) => setNewMemberUserId(e.target.value)}
              />
            </div>
          </div>
          <DialogFooter>
            <Button
              variant="outline"
              onClick={() => {
                setAddMemberOpen(false);
                setNewMemberUserId("");
              }}
            >
              Cancel
            </Button>
            <Button
              onClick={() => void handleAddMember()}
              isLoading={addMemberMutation.isPending}
            >
              Add Member
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      {/* Remove Member Confirmation */}
      <Dialog
        open={removeMemberId !== null}
        onOpenChange={(open) => {
          if (!open) setRemoveMemberId(null);
        }}
      >
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Remove Member</DialogTitle>
            <DialogDescription>
              Are you sure you want to remove this member from the group?
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button variant="outline" onClick={() => setRemoveMemberId(null)}>
              Cancel
            </Button>
            <Button
              variant="destructive"
              onClick={() => void handleRemoveMember()}
              isLoading={removeMemberMutation.isPending}
            >
              Remove Member
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}
