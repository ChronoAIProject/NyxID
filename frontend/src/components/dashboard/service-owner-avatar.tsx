import { UserRound } from "lucide-react";
import { OrgAvatar } from "@/components/orgs/org-avatar";
import { NyxidIcon } from "@/components/brand/nyxid-icon";
import { cn } from "@/lib/utils";

export function ServiceOwnerAvatar({
  type,
  name,
  avatarUrl,
  className,
}: {
  readonly type: "org" | "personal" | "platform";
  readonly name: string;
  readonly avatarUrl?: string | null;
  readonly className?: string;
}) {
  if (type === "org")
    return (
      <OrgAvatar
        displayName={name}
        avatarUrl={avatarUrl}
        className={cn(
          "size-5 rounded-full [&>*]:rounded-full [&>*]:text-[10px]",
          className,
        )}
      />
    );
  return (
    <span
      aria-label={name}
      className={cn(
        "flex size-5 shrink-0 items-center justify-center overflow-hidden rounded-full border border-border bg-muted/60",
        className,
      )}
    >
      {type === "platform" ? (
        <NyxidIcon className="size-3.5" alt="NyxID platform" />
      ) : (
        <UserRound
          className="size-3 text-muted-foreground"
          aria-hidden="true"
        />
      )}
    </span>
  );
}
