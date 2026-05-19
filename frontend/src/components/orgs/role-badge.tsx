import { Badge } from "@/components/ui/badge";
import type { OrgRole } from "@/schemas/orgs";

interface RoleBadgeProps {
  readonly role: OrgRole;
  readonly className?: string;
}

const ROLE_LABEL: Record<string, string> = {
  admin: "Admin",
  member: "Member",
  viewer: "Viewer",
  owner: "Owner",
};

/**
 * Small pill showing an org role. Colors follow the shared Badge variants so
 * they adapt to light/dark themes.
 */
export function RoleBadge({ role, className }: RoleBadgeProps) {
  const label = ROLE_LABEL[role] ?? role;
  if (!label) return null;

  const variant: "accent" | "info" | "success" | "secondary" =
    role === "owner"
      ? "accent"
      : role === "admin"
        ? "info"
        : role === "member"
          ? "success"
          : "secondary";

  return (
    <Badge variant={variant} className={className}>
      {label}
    </Badge>
  );
}
