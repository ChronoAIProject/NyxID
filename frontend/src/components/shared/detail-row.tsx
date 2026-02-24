import { copyToClipboard } from "@/lib/utils";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Copy } from "lucide-react";
import { toast } from "sonner";

interface DetailRowProps {
  readonly label: string;
  readonly value: string;
  readonly copyable?: boolean;
  readonly mono?: boolean;
  readonly badge?: boolean;
  readonly badgeVariant?:
    | "default"
    | "secondary"
    | "destructive"
    | "outline"
    | "success"
    | "warning";
}

/* ── VoidPortal Detail Row ── */
export function DetailRow({
  label,
  value,
  copyable = false,
  mono = false,
  badge = false,
  badgeVariant = "secondary",
}: DetailRowProps) {
  return (
    <div className="flex items-center justify-between border-b border-border py-2 text-sm last:border-b-0">
      <span className="text-text-tertiary">{label}</span>
      <div className="flex items-center gap-1">
        {badge ? (
          <Badge variant={badgeVariant}>{value}</Badge>
        ) : (
          <span className={mono ? "font-mono text-xs text-foreground" : "text-foreground"}>
            {value}
          </span>
        )}
        {copyable && (
          <Button
            variant="ghost"
            size="icon"
            className="h-6 w-6"
            onClick={() =>
              void copyToClipboard(value).then(
                () => toast.success(`${label} copied`),
                () => toast.error("Failed to copy"),
              )
            }
          >
            <Copy className="h-3 w-3" />
          </Button>
        )}
      </div>
    </div>
  );
}
