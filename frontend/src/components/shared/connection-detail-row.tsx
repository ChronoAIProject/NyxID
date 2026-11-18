import { cn } from "@/lib/utils";

/** Label/value row in a connection request summary. */
export function ConnectLinkDetailRow({
  label,
  value,
  capitalizeValue = false,
}: {
  readonly label: string;
  readonly value: string;
  readonly capitalizeValue?: boolean;
}) {
  return (
    <div className="flex justify-between gap-4 px-4 py-1 text-13">
      <span className="text-muted-foreground">{label}</span>
      <span
        className={cn(
          "min-w-0 break-words text-right font-medium text-foreground",
          capitalizeValue && "capitalize",
        )}
      >
        {value}
      </span>
    </div>
  );
}
