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
  const btnSize = size === "sm" ? "h-6 w-6" : "h-7 w-7";
  const padding = size === "sm" ? "px-2 py-1" : "px-2 py-1.5";

  return (
    <div>
      <p className={`mb-1 ${labelSize} font-medium text-muted-foreground`}>
        {label}
      </p>
      <div className="flex items-center gap-1">
        <code
          className={`flex-1 rounded bg-muted ${padding} ${textSize} break-all`}
        >
          {value}
        </code>
        <Button
          variant="ghost"
          size="icon"
          className={`${btnSize} shrink-0`}
          onClick={() => void handleCopy()}
        >
          {copied ? (
            <Check className="h-3 w-3 text-green-400" />
          ) : (
            <Copy className="h-3 w-3" />
          )}
          <span className="sr-only">Copy {label}</span>
        </Button>
      </div>
    </div>
  );
}
