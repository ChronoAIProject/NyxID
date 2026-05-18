import { useState } from "react";
import { Link, useNavigate } from "@tanstack/react-router";
import { Building2, Users } from "lucide-react";
import { HierarchyIcon } from "@/components/icons/empty-state";
import { AddCtaButton } from "@/components/shared/add-cta-button";
import { ErrorBanner } from "@/components/shared/error-banner";
import { ViewToggle, useViewMode } from "@/components/shared/view-toggle";
import { Card, CardContent } from "@/components/ui/card";
import { Skeleton } from "@/components/ui/skeleton";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { PageHeader } from "@/components/shared/page-header";
import { formatRelativeTime } from "@/lib/utils";
import { useOrgs } from "@/hooks/use-orgs";
import { CreateOrgDialog } from "@/components/orgs/create-org-dialog";
import { OrgAvatar } from "@/components/orgs/org-avatar";
import { RoleBadge } from "@/components/orgs/role-badge";

function OrgsLoading() {
  return (
    <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
      {Array.from({ length: 3 }).map((_, i) => (
        <Skeleton key={`org-skel-${String(i)}`} className="h-28 rounded-xl" />
      ))}
    </div>
  );
}

function OrgsEmptyState({ onCreate }: { readonly onCreate: () => void }) {
  return (
    <div className="flex flex-col items-center justify-center gap-1 py-12 text-center">
      <HierarchyIcon className="h-64 w-64 text-muted-foreground/30" />
      <div className="space-y-1">
        <p className="text-[12px] font-medium text-muted-foreground/30">No organizations yet</p>
        <p className="text-xs text-muted-foreground/30">
          Create an organization to share services and credentials with
          teammates.
        </p>
      </div>
      <AddCtaButton label="Create your first organization" onClick={onCreate} />
    </div>
  );
}

export function OrgsPage() {
  const { data: orgs, isLoading, error, refetch } = useOrgs();
  const [createOpen, setCreateOpen] = useState(false);
  const [viewMode, setViewMode] = useViewMode("orgs");
  const navigate = useNavigate();

  return (
    <div className="space-y-8">
      <PageHeader
        title="Organizations"
        description="Organizations let you share services and credentials with a team."
        actions={
          <div className="flex items-center gap-2">
            <ViewToggle viewMode={viewMode} onViewModeChange={setViewMode} />
            <AddCtaButton label="New Organization" onClick={() => setCreateOpen(true)} />
          </div>
        }
      />

      {isLoading ? (
        <OrgsLoading />
      ) : error ? (
        <ErrorBanner message="Failed to load organizations. Please try again." onRetry={refetch} />
      ) : !orgs || orgs.length === 0 ? (
        <OrgsEmptyState onCreate={() => setCreateOpen(true)} />
      ) : (
        <div className="space-y-3">
          <div className="flex items-center gap-2">
            <Building2 className="h-4 w-4 text-muted-foreground" />
            <h3 className="text-[13px] font-semibold text-foreground">My Organizations</h3>
          </div>

          {/* Cards - always on mobile, desktop only in grid mode */}
          <div className={`grid gap-4 sm:grid-cols-2 lg:grid-cols-3 ${viewMode === "table" ? "md:hidden" : ""}`}>
            {orgs.map((org) => (
              <Link
                key={org.id}
                to="/orgs/$orgId"
                params={{ orgId: org.id }}
                className="block focus-visible:outline-none rounded-xl"
              >
                <Card className="transition-colors duration-300 hover:border-white/[0.15] hover:bg-accent/30">
                  <CardContent className="flex flex-col gap-3 p-4">
                    <div className="flex items-start justify-between gap-2">
                      <div className="flex min-w-0 items-center gap-3">
                        <OrgAvatar
                          avatarUrl={org.avatar_url}
                          displayName={org.display_name}
                          className="h-8 w-8"
                        />
                        <div className="min-w-0">
                          <p className="truncate text-[12px] font-medium text-foreground">
                            {org.display_name ?? "Untitled org"}
                          </p>
                          <p className="truncate text-xs text-muted-foreground">
                            @{org.slug}
                          </p>
                          {org.contact_email && (
                            <p className="truncate text-xs text-muted-foreground">
                              {org.contact_email}
                            </p>
                          )}
                          <p className="text-xs text-muted-foreground">
                            Created {formatRelativeTime(org.created_at) ?? "—"}
                          </p>
                        </div>
                      </div>
                      <RoleBadge role={org.your_role} />
                    </div>
                    <div className="flex items-center gap-2 text-xs text-muted-foreground">
                      <Users className="h-3.5 w-3.5" />
                      <span>View members and invites</span>
                    </div>
                  </CardContent>
                </Card>
              </Link>
            ))}
          </div>

          {/* Table - desktop only in table mode */}
          {viewMode === "table" && (
            <div className="hidden md:block rounded-xl border border-border/50 bg-card overflow-hidden">
              <Table>
                <TableHeader>
                  <TableRow>
                    <TableHead>Name</TableHead>
                    <TableHead>Slug</TableHead>
                    <TableHead>Contact</TableHead>
                    <TableHead>Role</TableHead>
                    <TableHead>Created</TableHead>
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {orgs.map((org) => (
                    <TableRow
                      key={org.id}
                      className="cursor-pointer"
                      onClick={() => navigate({ to: "/orgs/$orgId", params: { orgId: org.id } })}
                    >
                      <TableCell>
                        <div className="flex items-center gap-2.5">
                          <OrgAvatar
                            avatarUrl={org.avatar_url}
                            displayName={org.display_name}
                            className="h-7 w-7"
                          />
                          <span className="truncate font-medium">
                            {org.display_name ?? "Untitled org"}
                          </span>
                        </div>
                      </TableCell>
                      <TableCell className="text-muted-foreground">@{org.slug}</TableCell>
                      <TableCell className="text-muted-foreground">
                        {org.contact_email ?? "—"}
                      </TableCell>
                      <TableCell>
                        <RoleBadge role={org.your_role} />
                      </TableCell>
                      <TableCell className="text-muted-foreground">
                        {formatRelativeTime(org.created_at) ?? "—"}
                      </TableCell>
                    </TableRow>
                  ))}
                </TableBody>
              </Table>
            </div>
          )}
        </div>
      )}

      <CreateOrgDialog open={createOpen} onOpenChange={setCreateOpen} />
    </div>
  );
}
