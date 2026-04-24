import { useState } from "react";
import { useParams, useNavigate } from "@tanstack/react-router";
import { useForm } from "react-hook-form";
import { zodResolver } from "@hookform/resolvers/zod";
import { Building2, Check, Copy, Plus, Trash2 } from "lucide-react";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { Skeleton } from "@/components/ui/skeleton";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import {
  Table,
  TableBody,
  TableHead,
  TableHeader,
  TableRow,
  TableCell,
} from "@/components/ui/table";
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
import { Badge } from "@/components/ui/badge";
import { PageHeader } from "@/components/shared/page-header";
import { ApiError } from "@/lib/api-client";
import { buildOrgInviteJoinUrl } from "@/lib/org-invite-links";
import {
  copyToClipboard,
  formatDate,
  formatRelativeTime,
  formatTimeDistance,
} from "@/lib/utils";
import { useAuthStore } from "@/stores/auth-store";
import {
  useOrg,
  useUpdateOrg,
  useDeleteOrg,
} from "@/hooks/use-orgs";
import {
  useOrgMembers,
  useUpdateMember,
  useRemoveMember,
} from "@/hooks/use-org-members";
import {
  useOrgInvites,
  useCancelInvite,
} from "@/hooks/use-org-invites";
import {
  updateOrgRequestSchema,
  type MemberResponse,
  type OrgRole,
  type UpdateOrgRequest,
} from "@/schemas/orgs";
import { MemberRow } from "@/components/orgs/member-row";
import { MemberScopeDialog } from "@/components/orgs/member-scope-dialog";
import { RoleBadge } from "@/components/orgs/role-badge";
import { InviteDialog } from "@/components/orgs/invite-dialog";
import { OrgApprovalConfigs } from "@/components/orgs/org-approval-configs";
import { OrgAvatar } from "@/components/orgs/org-avatar";

type TabValue = "members" | "invites" | "approvals" | "settings";

export function OrgDetailPage() {
  const { orgId } = useParams({ strict: false }) as { orgId: string };
  const navigate = useNavigate();
  const currentUser = useAuthStore((s) => s.user);

  const { data: org, isLoading, error } = useOrg(orgId);
  const { data: members, isLoading: membersLoading } = useOrgMembers(orgId);
  const { data: invites, isLoading: invitesLoading } = useOrgInvites(orgId);

  const updateMemberMutation = useUpdateMember();
  const removeMemberMutation = useRemoveMember();
  const cancelInviteMutation = useCancelInvite();
  const deleteOrgMutation = useDeleteOrg();

  const [tab, setTab] = useState<TabValue>("members");
  const [inviteOpen, setInviteOpen] = useState(false);
  const [revokeTarget, setRevokeTarget] = useState<MemberResponse | null>(null);
  const [scopeTarget, setScopeTarget] = useState<MemberResponse | null>(null);
  const [deleteOpen, setDeleteOpen] = useState(false);
  const [copiedInviteId, setCopiedInviteId] = useState<string | null>(null);

  if (isLoading) {
    return (
      <div className="space-y-6">
        <Skeleton className="h-10 w-64" />
        <Skeleton className="h-48 w-full" />
      </div>
    );
  }

  if (error || !org) {
    return (
      <Card>
        <CardContent className="flex flex-col items-center gap-4 py-16">
          <Building2 className="h-12 w-12 text-muted-foreground/50" />
          <p className="text-sm text-muted-foreground">
            Failed to load organization. It may have been deleted or you no
            longer have access.
          </p>
          <Button variant="outline" onClick={() => void navigate({ to: "/orgs" })}>
            Back to organizations
          </Button>
        </CardContent>
      </Card>
    );
  }

  const isAdmin = org.your_role === "admin";

  async function handleChangeRole(memberUserId: string, nextRole: OrgRole) {
    try {
      await updateMemberMutation.mutateAsync({
        orgId,
        memberId: memberUserId,
        body: { role: nextRole },
      });
      toast.success("Role updated");
    } catch (err) {
      toast.error(
        err instanceof ApiError ? err.message : "Failed to update role",
      );
    }
  }

  async function handleRevokeMember() {
    if (!revokeTarget) return;
    try {
      await removeMemberMutation.mutateAsync({
        orgId,
        memberId: revokeTarget.user_id,
      });
      toast.success("Member removed");
    } catch (err) {
      toast.error(
        err instanceof ApiError ? err.message : "Failed to remove member",
      );
    } finally {
      setRevokeTarget(null);
    }
  }

  async function handleCancelInvite(inviteId: string) {
    try {
      await cancelInviteMutation.mutateAsync({ orgId, inviteId });
      toast.success("Invite cancelled");
    } catch (err) {
      toast.error(
        err instanceof ApiError ? err.message : "Failed to cancel invite",
      );
    }
  }

  async function handleCopyInviteLink(inviteId: string, nonce: string) {
    try {
      await copyToClipboard(buildOrgInviteJoinUrl(nonce));
      setCopiedInviteId(inviteId);
      toast.success("Invite link copied");
      setTimeout(() => {
        setCopiedInviteId((current) => (current === inviteId ? null : current));
      }, 2000);
    } catch {
      toast.error("Failed to copy to clipboard");
    }
  }

  async function handleDeleteOrg() {
    try {
      await deleteOrgMutation.mutateAsync(orgId);
      toast.success("Organization deleted");
      void navigate({ to: "/orgs" });
    } catch (err) {
      toast.error(
        err instanceof ApiError ? err.message : "Failed to delete organization",
      );
    }
  }

  return (
    <div className="space-y-8">
      <PageHeader
        breadcrumbs={[
          { label: "Organizations", to: "/orgs" },
          { label: org.display_name ?? "Untitled org" },
        ]}
        title={org.display_name ?? "Untitled org"}
        description={
          org.contact_email
            ? `${org.contact_email} · ${String(org.member_count)} member${org.member_count === 1 ? "" : "s"}`
            : `${String(org.member_count)} member${org.member_count === 1 ? "" : "s"}`
        }
        leading={
          <OrgAvatar
            avatarUrl={org.avatar_url}
            displayName={org.display_name}
            className="h-14 w-14"
          />
        }
        actions={<RoleBadge role={org.your_role} />}
      />

      <Tabs value={tab} onValueChange={(value) => setTab(value as TabValue)}>
        <TabsList>
          <TabsTrigger value="members">Members</TabsTrigger>
          <TabsTrigger value="invites">Invites</TabsTrigger>
          <TabsTrigger value="approvals">Approvals</TabsTrigger>
          <TabsTrigger value="settings">Settings</TabsTrigger>
        </TabsList>

        <TabsContent value="members" className="mt-6 space-y-4">
          {isAdmin && (
            <div className="flex justify-end">
              <Button size="sm" onClick={() => setInviteOpen(true)}>
                <Plus className="mr-2 h-4 w-4" />
                Invite member
              </Button>
            </div>
          )}

          {membersLoading ? (
            <Skeleton className="h-40 w-full" />
          ) : !members || members.length === 0 ? (
            <Card>
              <CardContent className="py-8 text-center text-sm text-muted-foreground">
                No members yet.
              </CardContent>
            </Card>
          ) : (
            <div className="rounded-xl border border-border">
              <Table>
                <TableHeader>
                  <TableRow>
                    <TableHead>Member</TableHead>
                    <TableHead>Role</TableHead>
                    <TableHead>Services</TableHead>
                    <TableHead>Joined</TableHead>
                    <TableHead className="w-[100px] text-right">
                      <span className="sr-only">Actions</span>
                    </TableHead>
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {members.map((member) => (
                    <MemberRow
                      key={member.membership_id}
                      member={member}
                      canManage={isAdmin}
                      isSelf={member.user_id === currentUser?.id}
                      isUpdating={
                        updateMemberMutation.isPending ||
                        removeMemberMutation.isPending
                      }
                      onChangeRole={(id, role) => void handleChangeRole(id, role)}
                      onRevoke={(target) => setRevokeTarget(target)}
                      onEditScope={(target) => setScopeTarget(target)}
                    />
                  ))}
                </TableBody>
              </Table>
            </div>
          )}
        </TabsContent>

        <TabsContent value="invites" className="mt-6 space-y-4">
          {isAdmin ? (
            <div className="flex justify-end">
              <Button size="sm" onClick={() => setInviteOpen(true)}>
                <Plus className="mr-2 h-4 w-4" />
                Create invite
              </Button>
            </div>
          ) : (
            <Card>
              <CardContent className="py-6 text-center text-sm text-muted-foreground">
                Only admins can manage invites.
              </CardContent>
            </Card>
          )}

          {isAdmin && (
            <>
              {invitesLoading ? (
                <Skeleton className="h-40 w-full" />
              ) : !invites || invites.length === 0 ? (
                <Card>
                  <CardContent className="py-8 text-center text-sm text-muted-foreground">
                    No pending invites.
                  </CardContent>
                </Card>
              ) : (
                <div className="rounded-xl border border-border">
                  <Table>
                    <TableHeader>
                      <TableRow>
                        <TableHead>Nonce</TableHead>
                        <TableHead>Role</TableHead>
                        <TableHead>Status</TableHead>
                        <TableHead>Used by</TableHead>
                        <TableHead>Timeline</TableHead>
                        <TableHead className="w-[112px]" />
                      </TableRow>
                    </TableHeader>
                    <TableBody>
                      {invites.map((invite) => {
                        const isRedeemed = invite.redeemed_at !== null;
                        const isExpired =
                          !isRedeemed &&
                          new Date(invite.expires_at).getTime() < Date.now();
                        // For redeemed rows, show the user that consumed the
                        // invite (issue #409). Prefer email because that's
                        // the primary auth identifier; fall back to display
                        // name, then raw user id, then a dash.
                        const usedBy = isRedeemed
                          ? (invite.redeemed_by_email ??
                            invite.redeemed_by_display_name ??
                            invite.redeemed_by ??
                            "-")
                          : "-";
                        // Status-aware timeline text (issue #408): pending
                        // rows count down to expiry, expired/redeemed rows
                        // count up from the lifecycle event that actually
                        // ended the invite's usefulness.
                        let timeline: string;
                        let timelineTooltip: string | undefined;
                        if (isRedeemed && invite.redeemed_at) {
                          timeline = `Redeemed ${formatRelativeTime(invite.redeemed_at)}`;
                          timelineTooltip = `Original expiry ${formatDate(invite.expires_at)}`;
                        } else if (isExpired) {
                          timeline = `Expired ${formatRelativeTime(invite.expires_at)}`;
                        } else {
                          timeline = `Expires ${formatTimeDistance(invite.expires_at)}`;
                        }
                        return (
                          <TableRow key={invite.id}>
                            <TableCell>
                              <span className="break-all font-mono text-xs text-foreground">
                                {invite.nonce}
                              </span>
                            </TableCell>
                            <TableCell>
                              <RoleBadge role={invite.role} />
                            </TableCell>
                            <TableCell>
                              {isRedeemed ? (
                                <Badge variant="success">Redeemed</Badge>
                              ) : isExpired ? (
                                <Badge variant="warning">Expired</Badge>
                              ) : (
                                <Badge variant="info">Pending</Badge>
                              )}
                            </TableCell>
                            <TableCell className="break-all text-xs text-muted-foreground">
                              {usedBy}
                            </TableCell>
                            <TableCell
                              className="text-muted-foreground"
                              title={timelineTooltip}
                            >
                              {timeline}
                            </TableCell>
                            <TableCell>
                              {!isRedeemed && !isExpired && (
                                <div className="flex items-center justify-end gap-1">
                                  <Button
                                    variant="ghost"
                                    size="icon"
                                    className="h-8 w-8 text-muted-foreground hover:text-foreground"
                                    onClick={() =>
                                      void handleCopyInviteLink(
                                        invite.id,
                                        invite.nonce,
                                      )
                                    }
                                    aria-label={`Copy invite link ${invite.nonce}`}
                                    title="Copy invite link"
                                  >
                                    {copiedInviteId === invite.id ? (
                                      <Check className="h-4 w-4 text-success" />
                                    ) : (
                                      <Copy className="h-4 w-4" />
                                    )}
                                  </Button>
                                  <Button
                                    variant="ghost"
                                    size="icon"
                                    className="h-8 w-8 text-muted-foreground hover:text-destructive"
                                    onClick={() =>
                                      void handleCancelInvite(invite.id)
                                    }
                                    aria-label="Cancel invite"
                                  >
                                    <Trash2 className="h-4 w-4" />
                                  </Button>
                                </div>
                              )}
                            </TableCell>
                          </TableRow>
                        );
                      })}
                    </TableBody>
                  </Table>
                </div>
              )}
            </>
          )}
        </TabsContent>

        <TabsContent value="approvals" className="mt-6">
          {isAdmin ? (
            <OrgApprovalConfigs orgId={orgId} />
          ) : (
            <Card>
              <CardContent className="py-6 text-center text-sm text-muted-foreground">
                Only admins can manage org approval policies.
              </CardContent>
            </Card>
          )}
        </TabsContent>

        <TabsContent value="settings" className="mt-6">
          {isAdmin ? (
            <SettingsPanel
              orgId={orgId}
              initialDisplayName={org.display_name ?? ""}
              initialAvatarUrl={org.avatar_url ?? ""}
              initialContactEmail={org.contact_email ?? ""}
              onDelete={() => setDeleteOpen(true)}
            />
          ) : (
            <Card>
              <CardContent className="py-6 text-center text-sm text-muted-foreground">
                Only admins can edit organization settings.
              </CardContent>
            </Card>
          )}
        </TabsContent>
      </Tabs>

      <InviteDialog
        orgId={orgId}
        open={inviteOpen}
        onOpenChange={setInviteOpen}
      />

      <MemberScopeDialog
        orgId={orgId}
        member={scopeTarget}
        onOpenChange={(open) => {
          if (!open) setScopeTarget(null);
        }}
      />

      <Dialog
        open={revokeTarget !== null}
        onOpenChange={(open) => {
          if (!open) setRevokeTarget(null);
        }}
      >
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Remove member</DialogTitle>
            <DialogDescription>
              Remove{" "}
              <span className="font-medium text-foreground">
                {revokeTarget?.display_name ??
                  revokeTarget?.email ??
                  revokeTarget?.user_id ??
                  ""}
              </span>{" "}
              from this organization? They will lose access to shared services
              immediately.
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button variant="outline" onClick={() => setRevokeTarget(null)}>
              Cancel
            </Button>
            <Button
              variant="destructive"
              onClick={() => void handleRevokeMember()}
              isLoading={removeMemberMutation.isPending}
            >
              Remove
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <Dialog open={deleteOpen} onOpenChange={setDeleteOpen}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Delete organization</DialogTitle>
            <DialogDescription>
              Delete{" "}
              <span className="font-medium text-foreground">
                {org.display_name ?? "this organization"}
              </span>
              ? All memberships and invites will be removed. Shared services
              remain in place so admins can rescue their credentials. This
              action cannot be undone.
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button variant="outline" onClick={() => setDeleteOpen(false)}>
              Cancel
            </Button>
            <Button
              variant="destructive"
              onClick={() => void handleDeleteOrg()}
              isLoading={deleteOrgMutation.isPending}
            >
              Delete organization
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}

interface SettingsPanelProps {
  readonly orgId: string;
  readonly initialDisplayName: string;
  readonly initialAvatarUrl: string;
  readonly initialContactEmail: string;
  readonly onDelete: () => void;
}

function SettingsPanel({
  orgId,
  initialDisplayName,
  initialAvatarUrl,
  initialContactEmail,
  onDelete,
}: SettingsPanelProps) {
  const updateMutation = useUpdateOrg();

  const form = useForm<UpdateOrgRequest>({
    resolver: zodResolver(updateOrgRequestSchema),
    defaultValues: {
      display_name: initialDisplayName,
      avatar_url: initialAvatarUrl,
      contact_email: initialContactEmail,
    },
  });

  async function onSubmit(data: UpdateOrgRequest) {
    // Only send contact_email when the user actually changed it. This avoids
    // clobbering the backend-side placeholder with an empty string on
    // every Save click, and keeps audit entries accurate.
    const body: UpdateOrgRequest = { ...data };
    if ((body.contact_email ?? "") === initialContactEmail) {
      delete body.contact_email;
    }
    try {
      await updateMutation.mutateAsync({ orgId, body });
      toast.success("Organization updated");
    } catch (err) {
      if (err instanceof ApiError) {
        form.setError("root", { message: err.message });
      } else {
        toast.error("Failed to update organization");
      }
    }
  }

  return (
    <div className="space-y-6">
      <Card>
        <CardContent className="p-6">
          <Form {...form}>
            <form
              onSubmit={(e) => void form.handleSubmit(onSubmit)(e)}
              className="space-y-4"
            >
              <FormField
                control={form.control}
                name="display_name"
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>Display name</FormLabel>
                    <FormControl>
                      <Input {...field} />
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

              <FormField
                control={form.control}
                name="contact_email"
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>Contact email</FormLabel>
                    <FormControl>
                      <Input
                        type="email"
                        placeholder="you@example.com"
                        {...field}
                      />
                    </FormControl>
                    <p className="text-xs text-muted-foreground">
                      Shown in admin surfaces and used as the org user's
                      identity. Leave blank to clear.
                    </p>
                    <FormMessage />
                  </FormItem>
                )}
              />

              {form.formState.errors.root && (
                <p className="text-sm text-destructive">
                  {form.formState.errors.root.message}
                </p>
              )}

              <div className="flex justify-end">
                <Button type="submit" isLoading={updateMutation.isPending}>
                  Save changes
                </Button>
              </div>
            </form>
          </Form>
        </CardContent>
      </Card>

      <Card>
        <CardContent className="flex flex-col gap-4 p-6">
          <div>
            <p className="text-sm font-medium text-foreground">Danger zone</p>
            <p className="text-xs text-muted-foreground">
              Deleting the organization removes memberships and invites. Shared
              services stay in place so admins can rescue credentials.
            </p>
          </div>
          <div className="flex justify-end">
            <Button variant="destructive" onClick={onDelete}>
              <Trash2 className="mr-2 h-4 w-4" />
              Delete organization
            </Button>
          </div>
        </CardContent>
      </Card>
    </div>
  );
}
