import { useState } from "react";
import { Check, Copy } from "lucide-react";
import { Button } from "@/components/ui/button";
import { copyToClipboard } from "@/lib/utils";
import { toast } from "sonner";

interface CopyableFieldProps {
  readonly label: string;
  readonly value: string;
  readonly size?: "sm" | "md";
}

/* ── NyxID Copyable Field ── */
export function CopyableField({
  label,
  value,
  size = "md",
}: CopyableFieldProps) {
  const [copied, setCopied] = useState(false);

  async function handleCopy() {
    try {
      await copyToClipboard(value);
      setCopied(true);
      toast.success(`${label} copied`);
      setTimeout(() => setCopied(false), 2000);
    } catch {
      toast.error("Failed to copy");
    }
  }

  const textSize = size === "sm" ? "text-[10px]" : "text-xs";
  const labelSize = size === "sm" ? "text-[10px]" : "text-xs";
  const btnSize = size === "sm" ? "h-7 w-7" : "h-8 w-8";
  const padding = size === "sm" ? "px-2 py-1" : "px-2 py-1.5";

  return (
    <div>
      <p className={`mb-1 ${labelSize} font-medium text-text-tertiary`}>
        {label}
      </p>
      <div className="relative">
        <code
          className={`block rounded-xl border border-border bg-muted font-mono ${padding} pr-10 ${textSize} break-all text-foreground`}
        >
          {value}
        </code>
        <Button
          variant="ghost"
          size="icon"
          className={`${btnSize} absolute right-1 top-1/2 -translate-y-1/2 shrink-0`}
          onClick={() => void handleCopy()}
        >
          {copied ? (
            <Check className="h-3 w-3 text-success" />
          ) : (
            <Copy className="h-3 w-3" />
          )}
          <span className="sr-only">Copy {label}</span>
        </Button>
      </div>
    </div>
  );
}
