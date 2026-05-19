import { Building2 } from "lucide-react";
import { Avatar, AvatarFallback, AvatarImage } from "@/components/ui/avatar";
import { cn, sanitizeAvatarUrl } from "@/lib/utils";

interface OrgAvatarProps {
  readonly avatarUrl?: string | null;
  readonly displayName?: string | null;
  readonly className?: string;
}

/**
 * Derive 1-2 letter initials from an org display name. Falls back to the
 * building icon if the name is empty or contains no alphabetic characters
 * (e.g. pure-symbol names).
 */
function getInitials(displayName: string | null | undefined): string | null {
  if (!displayName) return null;
  const words = displayName
    .split(/[\s\-_/]+/)
    .map((w) => w.trim())
    .filter(Boolean);
  if (words.length === 0) return null;
  const initials = words
    .map((w) => w[0])
    .filter((c): c is string => typeof c === "string" && /[A-Za-z0-9]/.test(c))
    .join("")
    .slice(0, 2)
    .toUpperCase();
  return initials || null;
}

/**
 * Square-rounded avatar for an organization. Renders `avatar_url` when it
 * passes the standard sanitization check (http/https only). Falls back to
 * initials derived from the display name, or a building icon as a last
 * resort. Used in org cards, the org detail header, and anywhere else we
 * surface an org identity.
 */
export function OrgAvatar({
  avatarUrl,
  displayName,
  className,
}: OrgAvatarProps) {
  const safeUrl = sanitizeAvatarUrl(avatarUrl);
  const initials = getInitials(displayName);

  return (
    <Avatar
      className={cn("rounded-lg border-border bg-muted/60", className)}
      aria-label={displayName ?? "Organization"}
    >
      {safeUrl ? (
        <AvatarImage
          src={safeUrl}
          alt=""
          className="rounded-lg object-cover"
        />
      ) : null}
      <AvatarFallback className="rounded-lg text-muted-foreground">
        {initials ?? <Building2 className="h-1/2 w-1/2" aria-hidden="true" />}
      </AvatarFallback>
    </Avatar>
  );
}
