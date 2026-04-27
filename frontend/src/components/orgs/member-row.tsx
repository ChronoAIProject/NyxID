import { RotateCcw, SlidersHorizontal, Trash2 } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { TableCell, TableRow } from "@/components/ui/table";
import { formatRelativeTime } from "@/lib/utils";
import type { MemberResponse, OrgRole } from "@/schemas/orgs";
import { ORG_ROLES } from "@/schemas/orgs";
import { RoleBadge } from "./role-badge";

interface MemberRowProps {
  readonly member: MemberResponse;
  readonly canManage: boolean;
  readonly isSelf: boolean;
  readonly isUpdating: boolean;
  readonly onChangeRole: (memberId: string, nextRole: OrgRole) => void;
  readonly onRevoke: (member: MemberResponse) => void;
  readonly onEditScope: (member: MemberResponse) => void;
  readonly onResetScope: (member: MemberResponse) => void;
}

/**
 * Describe the member's current effective service scope in a short label.
 * `null` means "full access" (no restriction). Anything else is an
 * allow-list, whether inherited from the role or customized for the member.
 */
function scopeSummary(member: MemberResponse): string {
  const list = member.effective_allowed_service_ids;
  if (list === null) return "All services";
  if (list.length === 0) return "No services";
  return `${String(list.length)} service${list.length === 1 ? "" : "s"}`;
}

/**
 * A single row in the org members table. When the caller is not an admin
 * (`canManage === false`), the role is shown as a read-only badge and the
 * revoke button is hidden.
 */
export function MemberRow({
  member,
  canManage,
  isSelf,
  isUpdating,
  onChangeRole,
  onRevoke,
  onEditScope,
  onResetScope,
}: MemberRowProps) {
  const displayName =
    member.display_name ?? member.email ?? member.user_id;
  const scopeLabel = scopeSummary(member);
  const isScopeRestricted = member.effective_allowed_service_ids !== null;
  const hasCustomScope = member.scope_source === "override";

  return (
    <TableRow>
      <TableCell>
        <div className="flex flex-col gap-0.5">
          <span className="text-sm font-medium text-foreground">
            {displayName}
            {isSelf && (
              <span className="ml-2 text-xs text-muted-foreground">(you)</span>
            )}
            {hasCustomScope && (
              <Badge variant="info" className="ml-2 align-middle text-[11px]">
                Custom scope
              </Badge>
            )}
          </span>
          {member.email && member.display_name && (
            <span className="text-xs text-muted-foreground">
              {member.email}
            </span>
          )}
        </div>
      </TableCell>
      <TableCell>
        {canManage ? (
          <Select
            value={member.role}
            onValueChange={(next) => onChangeRole(member.user_id, next as OrgRole)}
            disabled={isUpdating}
          >
            <SelectTrigger className="h-8 w-[120px]">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {ORG_ROLES.map((role) => (
                <SelectItem key={role} value={role}>
                  {role.charAt(0).toUpperCase() + role.slice(1)}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        ) : (
          <RoleBadge role={member.role} />
        )}
      </TableCell>
      <TableCell>
        <Badge
          variant={isScopeRestricted ? "info" : "secondary"}
          className="text-xs"
        >
          {scopeLabel}
        </Badge>
      </TableCell>
      <TableCell className="text-muted-foreground">
        {formatRelativeTime(member.created_at) ?? "—"}
      </TableCell>
      <TableCell>
        <div className="flex items-center justify-end gap-1">
          {canManage && (
            <Button
              variant="ghost"
              size="icon"
              className="h-8 w-8 text-muted-foreground"
              onClick={() => onEditScope(member)}
              disabled={isUpdating}
              aria-label={`Edit service access for ${displayName}`}
              title="Edit service access"
            >
              <SlidersHorizontal className="h-4 w-4" />
            </Button>
          )}
          {canManage && hasCustomScope && (
            <Button
              variant="ghost"
              size="icon"
              className="h-8 w-8 text-muted-foreground"
              onClick={() => onResetScope(member)}
              disabled={isUpdating}
              aria-label={`Reset ${displayName} to role defaults`}
              title="Reset to role defaults"
            >
              <RotateCcw className="h-4 w-4" />
            </Button>
          )}
          {canManage && (
            <Button
              variant="ghost"
              size="icon"
              className="h-8 w-8 text-muted-foreground hover:text-destructive"
              onClick={() => onRevoke(member)}
              disabled={isUpdating}
              aria-label={`Remove ${displayName}`}
            >
              <Trash2 className="h-4 w-4" />
            </Button>
          )}
        </div>
      </TableCell>
    </TableRow>
  );
}
