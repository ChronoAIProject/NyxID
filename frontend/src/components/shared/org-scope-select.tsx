import { useOrgs } from "@/hooks/use-orgs";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";

const PERSONAL_VALUE = "__personal__";

interface OrgScopeSelectProps {
  /** Current scope: `null` means personal; a string is an org id. */
  readonly value: string | null;
  readonly onChange: (value: string | null) => void;
  readonly disabled?: boolean;
  /** Optional aria-label / test id override. */
  readonly label?: string;
  /** When true, only orgs where the caller is admin are offered. Defaults
   *  to true — the backend rejects create/list under an org for non-admins,
   *  so offering non-admin orgs in a create-time picker leads to 403s. */
  readonly adminOnly?: boolean;
}

/**
 * Select for choosing whether a resource should be created or listed under
 * the caller's personal scope, or under one of their orgs.
 *
 * Defaults to admin-only filtering because the backend rejects org-scoped
 * create/list calls from non-admins with 403. Pass `adminOnly={false}` if
 * you have a read-only context where viewers should also be able to pick
 * an org (there currently are none — the `/keys` list endpoint returns
 * org resources inline via membership, so the selector is only used on
 * create-admin paths).
 */
export function OrgScopeSelect({
  value,
  onChange,
  disabled,
  label = "Scope",
  adminOnly = true,
}: OrgScopeSelectProps) {
  const { data: orgs, isLoading } = useOrgs();

  const eligibleOrgs = (orgs ?? []).filter(
    (o) => !adminOnly || o.your_role === "admin",
  );

  return (
    <Select
      value={value ?? PERSONAL_VALUE}
      onValueChange={(next) =>
        onChange(next === PERSONAL_VALUE ? null : next)
      }
      disabled={disabled || isLoading}
    >
      <SelectTrigger aria-label={label}>
        <SelectValue placeholder="Personal" />
      </SelectTrigger>
      <SelectContent>
        <SelectItem value={PERSONAL_VALUE}>Personal</SelectItem>
        {eligibleOrgs.map((org) => (
          <SelectItem key={org.id} value={org.id}>
            {org.display_name || org.id}
          </SelectItem>
        ))}
      </SelectContent>
    </Select>
  );
}
