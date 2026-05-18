import { useState } from "react";
import { useParams, useNavigate, useRouterState } from "@tanstack/react-router";
import { useForm } from "react-hook-form";
import { zodResolver } from "@hookform/resolvers/zod";
import {
  Camera,
  Check,
  Copy,
  Globe,
  KeyRound,
  Trash2,
  X,
} from "lucide-react";
import { BenchesIcon, MailSendingIcon } from "@/components/icons/empty-state";
import { ErrorBanner } from "@/components/shared/error-banner";
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/components/ui/popover";
import { AddCtaButton } from "@/components/shared/add-cta-button";
import { toast } from "sonner";
import { Button, ButtonIcon } from "@/components/ui/button";
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
import { Checkbox } from "@/components/ui/checkbox";
import { Label } from "@/components/ui/label";
import { Switch } from "@/components/ui/switch";
import { PageHeader } from "@/components/shared/page-header";
import { useBreadcrumbLabel } from "@/components/layout/dashboard-layout";
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
  ORG_DETAIL_TABS,
  ORG_DETAIL_TAB_DEFAULT,
  type OrgDetailTab,
  parseTab,
} from "@/lib/url-tabs";
import { useOrg, useUpdateOrg, useDeleteOrg } from "@/hooks/use-orgs";
import {
  useOrgMembers,
  useUpdateMember,
  useRemoveMember,
} from "@/hooks/use-org-members";
import { useOrgInvites, useCancelInvite } from "@/hooks/use-org-invites";
import { useKeys } from "@/hooks/use-keys";
import {
  useClearOrgRoleScope,
  useOrgRoleScopes,
  useSetOrgRoleScope,
} from "@/hooks/use-org-role-scopes";
import {
  updateOrgRequestSchema,
  ORG_ROLES,
  type MemberResponse,
  type OrgRole,
  type UpdateOrgRequest,
} from "@/schemas/orgs";
import type { OrgRoleScope } from "@/schemas/org-role-scopes";
import type { KeyInfo } from "@/types/keys";
import { MemberRow } from "@/components/orgs/member-row";
import { MemberScopeDialog } from "@/components/orgs/member-scope-dialog";
import { RoleBadge } from "@/components/orgs/role-badge";
import { InviteDialog } from "@/components/orgs/invite-dialog";
import { OrgApprovalConfigs } from "@/components/orgs/org-approval-configs";
import { OrgAvatar } from "@/components/orgs/org-avatar";
import { OrgDeveloperAppsTab } from "@/components/orgs/org-developer-apps-tab";
import { OrgServiceAccountsTab } from "@/components/orgs/org-service-accounts-tab";

export function OrgDetailPage() {
  const { orgId } = useParams({ strict: false }) as { orgId: string };
  const navigate = useNavigate();
  const searchParams = useRouterState({ select: (s) => s.location.search as Record<string, unknown> });
  const tab = parseTab(searchParams.tab, ORG_DETAIL_TABS, ORG_DETAIL_TAB_DEFAULT);

  function setTab(value: OrgDetailTab) {
    void navigate({
      to: "/orgs/$orgId",
      params: { orgId },
      search: { tab: value },
      replace: true,
    });
  }

  const currentUser = useAuthStore((s) => s.user);

  const { data: org, isLoading, error, refetch } = useOrg(orgId);
  const { data: members, isLoading: membersLoading } = useOrgMembers(orgId);
  const { data: invites, isLoading: invitesLoading } = useOrgInvites(orgId);

  const updateMemberMutation = useUpdateMember();
  const removeMemberMutation = useRemoveMember();
  const cancelInviteMutation = useCancelInvite();
  const deleteOrgMutation = useDeleteOrg();

  useBreadcrumbLabel(org?.display_name);
  const [inviteOpen, setInviteOpen] = useState(false);
  const [revokeTarget, setRevokeTarget] = useState<MemberResponse | null>(null);
  const [scopeTarget, setScopeTarget] = useState<MemberResponse | null>(null);
  const [deleteOpen, setDeleteOpen] = useState(false);
  const [copiedInviteId, setCopiedInviteId] = useState<string | null>(null);
  const [avatarPopoverOpen, setAvatarPopoverOpen] = useState(false);
  const [avatarUrlInput, setAvatarUrlInput] = useState("");
  const avatarUpdateMutation = useUpdateOrg();

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
      <div className="space-y-4">
        <ErrorBanner message="Failed to load organization. It may have been deleted or you no longer have access." onRetry={refetch} />
        <Button
          variant="outline"
          onClick={() => void navigate({ to: "/orgs" })}
        >
          Back to organizations
        </Button>
      </div>
    );
  }

  const isAdmin = org.your_role === "admin" || org.your_role === "owner";
  const orgName = org.display_name ?? "Untitled org";
  const activeAdminCount = (members ?? []).filter(
    (member) => member.role === "admin" && member.revoked_at === null,
  ).length;

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

  async function handleResetMemberScope(member: MemberResponse) {
    try {
      await updateMemberMutation.mutateAsync({
        orgId,
        memberId: member.user_id,
        body: { scope_source: "inherit" },
      });
      toast.success("Member reset to role defaults");
    } catch (err) {
      toast.error(
        err instanceof ApiError ? err.message : "Failed to reset member scope",
      );
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
        title={org.display_name ?? "Untitled org"}
        description={
          org.contact_email
            ? `${org.contact_email} · ${String(org.member_count)} member${org.member_count === 1 ? "" : "s"}`
            : `${String(org.member_count)} member${org.member_count === 1 ? "" : "s"}`
        }
        leading={
          isAdmin ? (
            <Popover open={avatarPopoverOpen} onOpenChange={(open) => {
              setAvatarPopoverOpen(open);
              if (open) setAvatarUrlInput(org.avatar_url ?? "");
            }}>
              <PopoverTrigger asChild>
                <button type="button" className="group relative rounded-lg focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary/50">
                  <OrgAvatar avatarUrl={org.avatar_url} displayName={org.display_name} className="h-14 w-14" />
                  <div className="absolute inset-0 flex items-center justify-center rounded-lg bg-black/50 opacity-0 transition-opacity duration-200 group-hover:opacity-100">
                    <Camera className="h-5 w-5 text-white" />
                  </div>
                </button>
              </PopoverTrigger>
              <PopoverContent side="bottom" align="start" className="w-[280px] p-3 space-y-3">
                <p className="text-[12px] font-medium text-foreground">Change avatar</p>
                <div className="flex gap-2">
                  <Input
                    value={avatarUrlInput}
                    onChange={(e) => setAvatarUrlInput(e.target.value)}
                    placeholder="https://..."
                    className="flex-1"
                  />
                  <Button
                    variant="primary"
                    disabled={avatarUrlInput === (org.avatar_url ?? "")}
                    isLoading={avatarUpdateMutation.isPending}
                    onClick={async () => {
                      try {
                        await avatarUpdateMutation.mutateAsync({ orgId, body: { avatar_url: avatarUrlInput } });
                        toast.success("Avatar updated");
                        setAvatarPopoverOpen(false);
                      } catch {
                        toast.error("Failed to update avatar");
                      }
                    }}
                  >
                    Save
                  </Button>
                </div>
                {org.avatar_url && (
                  <button
                    type="button"
                    className="flex items-center gap-1 text-[11px] text-destructive hover:underline"
                    onClick={async () => {
                      try {
                        await avatarUpdateMutation.mutateAsync({ orgId, body: { avatar_url: "" } });
                        toast.success("Avatar removed");
                        setAvatarPopoverOpen(false);
                      } catch {
                        toast.error("Failed to remove avatar");
                      }
                    }}
                  >
                    <X className="h-3 w-3" />
                    Remove avatar
                  </button>
                )}
              </PopoverContent>
            </Popover>
          ) : (
            <OrgAvatar avatarUrl={org.avatar_url} displayName={org.display_name} className="h-14 w-14" />
          )
        }
        actions={<RoleBadge role={org.your_role} />}
      />

      <Tabs value={tab} onValueChange={(value) => setTab(value as OrgDetailTab)}>
        <TabsList>
          <TabsTrigger value="members">Members</TabsTrigger>
          <TabsTrigger value="role-permissions">Role permissions</TabsTrigger>
          <TabsTrigger value="invites">Invites</TabsTrigger>
          <TabsTrigger value="approvals">Approvals</TabsTrigger>
          {isAdmin && (
            <TabsTrigger value="service-accounts">Service Accounts</TabsTrigger>
          )}
          {isAdmin && (
            <TabsTrigger value="developer-apps">Developer Apps</TabsTrigger>
          )}
          <TabsTrigger value="settings">Settings</TabsTrigger>
        </TabsList>

        <TabsContent value="members" className="mt-6 space-y-4">
          {isAdmin && (
            <div className="flex justify-end">
              <AddCtaButton label="Invite Member" onClick={() => setInviteOpen(true)} />
            </div>
          )}

          {membersLoading ? (
            <Skeleton className="h-40 w-full" />
          ) : !members || members.length === 0 ? (
            <div className="flex flex-col items-center justify-center gap-1 py-12 text-center">
              <BenchesIcon className="h-64 w-64 text-muted-foreground/30" />
              <div className="space-y-1">
                <p className="text-[12px] font-medium text-muted-foreground/30">No Members</p>
                <p className="text-xs text-muted-foreground/30">No members yet.</p>
              </div>
            </div>
          ) : (
            <>
              {/* Mobile card view */}
              <div className="flex flex-col gap-3 md:hidden">
                {members.map((member) => {
                  const displayName = member.display_name ?? member.email ?? member.user_id;
                  const isSelf = member.user_id === currentUser?.id;
                  const scopeList = member.effective_allowed_service_ids;
                  const scopeLabel = scopeList == null ? "All services" : scopeList.length === 0 ? "No services" : `${String(scopeList.length)} service${scopeList.length === 1 ? "" : "s"}`;
                  return (
                    <div key={member.membership_id} className="relative rounded-xl border border-border/50 bg-card p-4">
                      {isAdmin && (
                        <div className="absolute right-3 top-3">
                          <Button
                            variant="ghost"
                            size="icon"
                            className="h-7 w-7"
                            onClick={() => setRevokeTarget(member)}
                            disabled={member.role === "admin" && member.revoked_at === null && activeAdminCount <= 1}
                          >
                            <Trash2 className="h-3.5 w-3.5 text-destructive" />
                          </Button>
                        </div>
                      )}
                      <p className="pr-10 text-[13px] font-semibold text-foreground truncate">
                        {displayName}
                        {isSelf && <span className="ml-1 text-[11px] font-normal text-muted-foreground">(you)</span>}
                      </p>
                      {member.email && member.display_name && (
                        <p className="text-[11px] text-muted-foreground truncate">{member.email}</p>
                      )}
                      <div className="mt-2 flex flex-wrap gap-1.5">
                        <RoleBadge role={member.role} />
                        <Badge variant={scopeList != null ? "info" : "secondary"} className="text-xs">{scopeLabel}</Badge>
                        {member.scope_source === "override" && <Badge variant="info" className="text-xs">Custom scope</Badge>}
                      </div>
                      <div className="mt-3 text-[11px] text-muted-foreground">
                        Joined {formatRelativeTime(member.created_at) ?? "—"}
                      </div>
                    </div>
                  );
                })}
              </div>

              {/* Desktop table view */}
              <div className="hidden md:block rounded-xl border border-border/50 bg-card overflow-hidden">
                <Table>
                  <TableHeader>
                    <TableRow>
                      <TableHead>Member</TableHead>
                      <TableHead>Role</TableHead>
                      <TableHead>Services</TableHead>
                      <TableHead>Joined</TableHead>
                      {isAdmin && (
                        <TableHead className="w-[100px] text-right">
                          Actions
                        </TableHead>
                      )}
                    </TableRow>
                  </TableHeader>
                  <TableBody>
                    {members.map((member) => (
                      <MemberRow
                        key={member.membership_id}
                        member={member}
                        canManage={isAdmin}
                        isSelf={member.user_id === currentUser?.id}
                        isLastAdmin={
                          member.role === "admin" &&
                          member.revoked_at === null &&
                          activeAdminCount <= 1
                        }
                        isUpdating={
                          updateMemberMutation.isPending ||
                          removeMemberMutation.isPending
                        }
                        onChangeRole={(id, role) =>
                          void handleChangeRole(id, role)
                        }
                        onRevoke={(target) => setRevokeTarget(target)}
                        onEditScope={(target) => setScopeTarget(target)}
                        onResetScope={(target) =>
                          void handleResetMemberScope(target)
                        }
                      />
                    ))}
                  </TableBody>
                </Table>
              </div>
            </>
          )}
        </TabsContent>

        <TabsContent value="role-permissions" className="mt-6">
          {isAdmin ? (
            <RolePermissionsPanel orgId={orgId} />
          ) : (
            <Card>
              <CardContent className="py-6 text-center text-[12px] text-muted-foreground">
                Only admins can manage role permissions.
              </CardContent>
            </Card>
          )}
        </TabsContent>

        <TabsContent value="invites" className="mt-6 space-y-4">
          {isAdmin ? (
            <div className="flex justify-end">
              <AddCtaButton label="Create Invite" onClick={() => setInviteOpen(true)} />
            </div>
          ) : (
            <Card>
              <CardContent className="py-6 text-center text-[12px] text-muted-foreground">
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
                  <CardContent className="flex flex-col items-center justify-center gap-1 py-8 text-center">
                    <MailSendingIcon className="h-48 w-48 text-muted-foreground/30" />
                    <p className="text-[12px] text-muted-foreground/30">No pending invites.</p>
                  </CardContent>
                </Card>
              ) : (
                <>
                  {/* Mobile card view */}
                  <div className="flex flex-col gap-3 md:hidden">
                    {invites.map((invite) => {
                      const isRedeemed = invite.redeemed_at !== null;
                      const isExpired = !isRedeemed && new Date(invite.expires_at).getTime() < Date.now();
                      const usedBy = isRedeemed ? (invite.redeemed_by_email ?? invite.redeemed_by_display_name ?? invite.redeemed_by ?? "—") : null;
                      let timeline: string;
                      if (isRedeemed && invite.redeemed_at) {
                        timeline = `Redeemed ${formatRelativeTime(invite.redeemed_at)}`;
                      } else if (isExpired) {
                        timeline = `Expired ${formatRelativeTime(invite.expires_at)}`;
                      } else {
                        timeline = `Expires ${formatTimeDistance(invite.expires_at)}`;
                      }
                      return (
                        <div key={invite.id} className="relative rounded-xl border border-border/50 bg-card p-4">
                          {!isRedeemed && !isExpired && (
                            <div className="absolute right-3 top-3 flex gap-1">
                              <Button variant="ghost" size="icon" className="h-7 w-7" onClick={() => void handleCopyInviteLink(invite.id, invite.nonce)} title="Copy invite link">
                                {copiedInviteId === invite.id ? <Check className="h-3.5 w-3.5 text-success" /> : <Copy className="h-3.5 w-3.5" />}
                              </Button>
                              <Button variant="ghost" size="icon" className="h-7 w-7" onClick={() => void handleCancelInvite(invite.id)}>
                                <Trash2 className="h-3.5 w-3.5 text-destructive" />
                              </Button>
                            </div>
                          )}
                          <p className="pr-20 text-[12px] font-mono text-foreground break-all">{invite.nonce}</p>
                          <div className="mt-2 flex flex-wrap gap-1.5">
                            <RoleBadge role={invite.role} />
                            {isRedeemed ? <Badge variant="success">Redeemed</Badge> : isExpired ? <Badge variant="warning">Expired</Badge> : <Badge variant="info">Pending</Badge>}
                          </div>
                          <div className="mt-3 flex flex-wrap gap-x-4 gap-y-1 text-[11px] text-muted-foreground">
                            <span>{timeline}</span>
                            {usedBy && <span>Used by {usedBy}</span>}
                          </div>
                        </div>
                      );
                    })}
                  </div>

                  {/* Desktop table view */}
                  <div className="hidden md:block rounded-xl border border-border/50 bg-card overflow-hidden">
                    <Table>
                      <TableHeader>
                        <TableRow>
                          <TableHead>Nonce</TableHead>
                          <TableHead>Role</TableHead>
                          <TableHead>Status</TableHead>
                          <TableHead>Used by</TableHead>
                          <TableHead>Timeline</TableHead>
                          {invites.some((inv) => inv.redeemed_at === null && new Date(inv.expires_at).getTime() >= Date.now()) && (
                            <TableHead className="w-[112px]">Actions</TableHead>
                          )}
                        </TableRow>
                      </TableHeader>
                      <TableBody>
                        {invites.map((invite) => {
                          const isRedeemed = invite.redeemed_at !== null;
                          const isExpired =
                            !isRedeemed &&
                            new Date(invite.expires_at).getTime() < Date.now();
                          const usedBy = isRedeemed
                            ? (invite.redeemed_by_email ??
                              invite.redeemed_by_display_name ??
                              invite.redeemed_by ??
                              "-")
                            : "-";
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
                                <span className="break-all text-xs text-foreground">
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
                              {invites.some((inv) => inv.redeemed_at === null && new Date(inv.expires_at).getTime() >= Date.now()) && (
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
                                        <Trash2 className="h-4 w-4 text-destructive" />
                                      </Button>
                                    </div>
                                  )}
                                </TableCell>
                              )}
                            </TableRow>
                          );
                        })}
                      </TableBody>
                    </Table>
                  </div>
                </>
              )}
            </>
          )}
        </TabsContent>

        <TabsContent value="approvals" className="mt-6">
          {isAdmin ? (
            <OrgApprovalConfigs orgId={orgId} />
          ) : (
            <Card>
              <CardContent className="py-6 text-center text-[12px] text-muted-foreground">
                Only admins can manage org approval policies.
              </CardContent>
            </Card>
          )}
        </TabsContent>

        {isAdmin && (
          <TabsContent value="service-accounts" className="mt-6">
            <OrgServiceAccountsTab orgId={orgId} orgName={orgName} />
          </TabsContent>
        )}

        {isAdmin && (
          <TabsContent value="developer-apps" className="mt-6">
            <OrgDeveloperAppsTab orgId={orgId} orgName={orgName} />
          </TabsContent>
        )}

        <TabsContent value="settings" className="mt-6">
          {isAdmin ? (
            <SettingsPanel
              orgId={orgId}
              initialDisplayName={org.display_name ?? ""}
              initialSlug={org.slug}
              initialAvatarUrl={org.avatar_url ?? ""}
              initialContactEmail={org.contact_email ?? ""}
              onDelete={() => setDeleteOpen(true)}
            />
          ) : (
            <Card>
              <CardContent className="py-6 text-center text-[12px] text-muted-foreground">
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
            <DialogTitle>Delete Organization</DialogTitle>
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
              Delete Organization
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}

function RolePermissionsPanel({ orgId }: { readonly orgId: string }) {
  const { data: keys, isLoading: keysLoading } = useKeys();
  const { data: roleScopes, isLoading: scopesLoading } =
    useOrgRoleScopes(orgId);
  const setScopeMutation = useSetOrgRoleScope(orgId);
  const clearScopeMutation = useClearOrgRoleScope(orgId);

  const orgServices = (keys ?? []).filter(
    (key) =>
      key.credential_source?.type === "org" &&
      key.credential_source.org_id === orgId,
  );
  const roleScopeMap = new Map(roleScopes?.map((scope) => [scope.role, scope]));
  const isLoading = keysLoading || scopesLoading;

  async function saveRoleScope(
    role: OrgRole,
    allowedServiceIds: readonly string[] | null,
  ) {
    try {
      if (allowedServiceIds === null) {
        await clearScopeMutation.mutateAsync({ role });
      } else {
        await setScopeMutation.mutateAsync({
          role,
          body: { allowed_service_ids: [...allowedServiceIds] },
        });
      }
      toast.success("Role permissions updated");
    } catch (err) {
      toast.error(
        err instanceof ApiError
          ? err.message
          : "Failed to update role permissions",
      );
    }
  }

  if (isLoading) {
    return <Skeleton className="h-64 w-full" />;
  }

  return (
    <div className="grid gap-4 lg:grid-cols-3">
      {ORG_ROLES.map((role) => {
        const scope = roleScopeMap.get(role) ?? defaultRoleScope(role);
        const pending =
          (setScopeMutation.isPending &&
            setScopeMutation.variables?.role === role) ||
          (clearScopeMutation.isPending &&
            clearScopeMutation.variables?.role === role);
        return (
          <RolePermissionCard
            // Key on the persisted scope so an external invalidation
            // (e.g. another admin saved) resets local draft state.
            key={`${role}-${scope.updated_at ?? "default"}`}
            role={role}
            scope={scope}
            services={orgServices}
            pending={pending}
            onSave={(next) => void saveRoleScope(role, next)}
          />
        );
      })}
    </div>
  );
}

interface RolePermissionCardProps {
  readonly role: OrgRole;
  readonly scope: OrgRoleScope;
  readonly services: readonly KeyInfo[];
  readonly pending: boolean;
  readonly onSave: (allowedServiceIds: readonly string[] | null) => void;
}

function RolePermissionCard({
  role,
  scope,
  services,
  pending,
  onSave,
}: RolePermissionCardProps) {
  const [draftFullAccess, setDraftFullAccess] = useState(
    scope.allowed_service_ids === null,
  );
  const [draftSelectedIds, setDraftSelectedIds] = useState<readonly string[]>(
    scope.allowed_service_ids ?? [],
  );

  const savedFullAccess = scope.allowed_service_ids === null;
  const savedSelectedIds = scope.allowed_service_ids ?? [];
  const dirty =
    draftFullAccess !== savedFullAccess ||
    (!draftFullAccess && !sameSet(draftSelectedIds, savedSelectedIds));

  function toggleService(serviceId: string) {
    if (draftFullAccess) return;
    setDraftSelectedIds((prev) =>
      prev.includes(serviceId)
        ? prev.filter((id) => id !== serviceId)
        : [...prev, serviceId],
    );
  }

  function reset() {
    setDraftFullAccess(savedFullAccess);
    setDraftSelectedIds(savedSelectedIds);
  }

  function save() {
    if (draftFullAccess) {
      onSave(null);
    } else {
      onSave([...draftSelectedIds]);
    }
  }

  return (
    <Card>
      <CardContent className="space-y-4 p-4">
        <div className="flex items-start justify-between gap-3">
          <div>
            <p className="text-[13px] font-semibold text-foreground">
              {roleLabel(role)}
            </p>
            <p className="text-xs text-muted-foreground">
              {scopeSummary(scope.allowed_service_ids)}
            </p>
          </div>
          {scope.is_default && (
            <Badge variant="secondary" className="text-[11px]">
              Default
            </Badge>
          )}
        </div>

        <div className="flex items-center justify-between gap-3 rounded-lg border border-border bg-muted/30 p-3">
          <Label
            htmlFor={`role-scope-full-${role}`}
            className="text-[12px] font-medium"
          >
            Full access
          </Label>
          <Switch
            id={`role-scope-full-${role}`}
            checked={draftFullAccess}
            disabled={pending}
            onCheckedChange={(checked) => setDraftFullAccess(checked === true)}
            aria-label={`Toggle full access for ${roleLabel(role)}`}
          />
        </div>

        <div className="space-y-2">
          <Label className="text-xs font-medium text-muted-foreground">
            Services
          </Label>
          {services.length === 0 ? (
            <div className="rounded-lg border border-dashed border-border p-4 text-center text-xs text-muted-foreground">
              This org has no services yet.
            </div>
          ) : (
            <div className="max-h-72 space-y-1 overflow-y-auto rounded-lg border border-border p-2">
              {services.map((service) => {
                const id = `role-scope-${role}-${service.id}`;
                const checked =
                  draftFullAccess || draftSelectedIds.includes(service.id);
                return (
                  <div
                    key={service.id}
                    className="flex items-start gap-3 rounded px-2 py-1.5 hover:bg-accent/40"
                  >
                    <Checkbox
                      id={id}
                      checked={checked}
                      disabled={pending || draftFullAccess}
                      onCheckedChange={() => toggleService(service.id)}
                      className="mt-1"
                    />
                    <Label
                      htmlFor={id}
                      className="flex-1 cursor-pointer space-y-0.5"
                    >
                      <span className="block text-[12px] font-medium text-foreground">
                        {service.label}
                      </span>
                      <span className="flex items-center gap-2 text-xs text-muted-foreground">
                        {service.service_type === "ssh" ? (
                          <KeyRound className="h-3 w-3" aria-hidden />
                        ) : (
                          <Globe className="h-3 w-3" aria-hidden />
                        )}
                        <span>{service.slug}</span>
                      </span>
                    </Label>
                  </div>
                );
              })}
            </div>
          )}
        </div>

        <div className="flex items-center justify-end gap-2 border-t border-border pt-3">
          <Button
            variant="ghost"
            onClick={reset}
            disabled={!dirty || pending}
          >
            Reset
          </Button>
          <Button
            variant="primary"
            onClick={save}
            disabled={!dirty}
            isLoading={pending}
          >
            Save
          </Button>
        </div>
      </CardContent>
    </Card>
  );
}

function sameSet(a: readonly string[], b: readonly string[]): boolean {
  if (a.length !== b.length) return false;
  const aSet = new Set(a);
  return b.every((id) => aSet.has(id));
}

function defaultRoleScope(role: OrgRole): OrgRoleScope {
  return {
    role,
    allowed_service_ids: null,
    is_default: true,
    updated_at: null,
    updated_by: null,
  };
}

function roleLabel(role: OrgRole): string {
  return role.charAt(0).toUpperCase() + role.slice(1);
}

function scopeSummary(allowedServiceIds: readonly string[] | null): string {
  if (allowedServiceIds === null) return "Full access";
  if (allowedServiceIds.length === 0) return "No services";
  return `${String(allowedServiceIds.length)} service${allowedServiceIds.length === 1 ? "" : "s"}`;
}

interface SettingsPanelProps {
  readonly orgId: string;
  readonly initialDisplayName: string;
  readonly initialSlug: string;
  readonly initialAvatarUrl: string;
  readonly initialContactEmail: string;
  readonly onDelete: () => void;
}

function SettingsPanel({
  orgId,
  initialDisplayName,
  initialSlug,
  initialAvatarUrl,
  initialContactEmail,
  onDelete,
}: SettingsPanelProps) {
  const updateMutation = useUpdateOrg();

  const form = useForm<UpdateOrgRequest>({
    resolver: zodResolver(updateOrgRequestSchema),
    defaultValues: {
      display_name: initialDisplayName,
      slug: initialSlug,
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
                name="slug"
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>Slug</FormLabel>
                    <FormControl>
                      <Input {...field} />
                    </FormControl>
                    <p className="text-xs text-muted-foreground">
                      URL-friendly identifier. Pass to --org or use in
                      slug-based API paths. Auto-generated from the display name
                      on org creation; admins can edit later.
                    </p>
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
                <p className="text-[12px] text-destructive">
                  {form.formState.errors.root.message}
                </p>
              )}

              <div className="flex justify-end">
                <Button variant="primary" type="submit" isLoading={updateMutation.isPending} disabled={!form.formState.isDirty}>
                  Save Changes
                </Button>
              </div>
            </form>
          </Form>
        </CardContent>
      </Card>

      <Card>
        <CardContent className="flex flex-col gap-4 p-4">
          <div>
            <p className="text-[12px] font-medium text-foreground">Danger zone</p>
            <p className="text-xs text-muted-foreground">
              Deleting the organization removes memberships and invites. Shared
              services stay in place so admins can rescue credentials.
            </p>
          </div>
          <div className="flex justify-end">
            <Button variant="destructive" onClick={onDelete}>
              <ButtonIcon variant="destructive"><Trash2 className="h-4 w-4 text-destructive" /></ButtonIcon>
              Delete Organization
            </Button>
          </div>
        </CardContent>
      </Card>
    </div>
  );
}
