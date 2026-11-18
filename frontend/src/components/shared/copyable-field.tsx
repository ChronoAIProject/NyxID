import { translateUi, useUiTranslation } from "@/i18n/use-ui-translation";

import { useState } from "react";
import { Check, Copy } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Label } from "@/components/ui/label";
import { copyToClipboard } from "@/lib/utils";
import { toast } from "sonner";

interface CopyableFieldProps {
  readonly label: string;
  readonly value: string;
  readonly size?: "sm" | "md";
  readonly grouped?: boolean;
}

/* ── NyxID Copyable Field ── */
export function CopyableField({
  label,
  value,
  size = "md",
  grouped = false,
}: CopyableFieldProps) {
  const { t } = useUiTranslation();
  const [copied, setCopied] = useState(false);

  async function handleCopy() {
    try {
      await copyToClipboard(value);
      setCopied(true);
      toast.success(translateUi("{{label}} copied", { label: label }));
      setTimeout(() => setCopied(false), 2000);
    } catch {
      toast.error(translateUi("Failed to copy"));
    }
  }

  const textSize = size === "sm" ? "type-code unthemed:text-10" : "type-code unthemed:text-11";
  const btnSize = size === "sm" ? "h-7 w-7" : "h-8 w-8";
  const padding = size === "sm" ? "px-3 py-2" : "px-4 py-3.5";
  const minH = size === "sm" ? "min-h-[36px]" : "min-h-[44px]";

  return (
    <div className={grouped ? "w-full" : undefined}>
      <Label asChild className={`mb-1 block text-text-tertiary ${size === "sm" ? "unthemed:text-10" : "unthemed:text-11"}`}>
        <p>{label}</p>
      </Label>
      <div className="relative">
        <code
          className={
            grouped
              ? "flex min-h-8 items-center break-all pr-10 font-mono type-code unthemed:text-11 text-foreground"
              : `flex items-center rounded-xl border border-border bg-muted font-mono ${padding} pr-10 ${minH} ${textSize} break-all text-foreground`
          }
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
          <span className="sr-only">{t("Copy ")}{label}</span>
        </Button>
      </div>
    </div>
  );
}
