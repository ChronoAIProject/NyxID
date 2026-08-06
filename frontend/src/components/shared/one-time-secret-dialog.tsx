import { Copy } from "lucide-react";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { copyToClipboard } from "@/lib/utils";

export interface OneTimeSecretValue {
  readonly label: string;
  readonly value: string;
}

interface OneTimeSecretDialogProps {
  readonly open: boolean;
  readonly onOpenChange: (open: boolean) => void;
  readonly title: string;
  readonly description: string;
  readonly values: readonly OneTimeSecretValue[];
}

export function OneTimeSecretDialog({
  open,
  onOpenChange,
  title,
  description,
  values,
}: OneTimeSecretDialogProps) {
  function copy(value: OneTimeSecretValue) {
    void copyToClipboard(value.value).then(
      () => toast.success(`${value.label} copied`),
      () => toast.error(`Failed to copy ${value.label.toLowerCase()}`),
    );
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>{title}</DialogTitle>
          <DialogDescription>{description}</DialogDescription>
        </DialogHeader>
        <div className="space-y-4">
          {values.map((value) => (
            <div key={value.label} className="space-y-2">
              <p className="text-[10px] font-medium uppercase tracking-[1.5px] text-text-tertiary">
                {value.label}
              </p>
              <div className="flex items-center gap-2 rounded-lg border border-border bg-muted px-3 py-2">
                <p className="min-w-0 flex-1 break-all font-mono text-xs">
                  {value.value}
                </p>
                <Button
                  type="button"
                  variant="ghost"
                  size="icon"
                  className="h-7 w-7 shrink-0"
                  aria-label={`Copy ${value.label}`}
                  onClick={() => copy(value)}
                >
                  <Copy className="h-3.5 w-3.5" />
                </Button>
              </div>
            </div>
          ))}
        </div>
        <DialogFooter>
          <Button
            type="button"
            variant="primary"
            onClick={() => onOpenChange(false)}
          >
            I have saved it
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
