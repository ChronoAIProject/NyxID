import { useState } from "react";
import type { DownstreamService } from "@/types/api";
import { getCredentialInputType } from "@/lib/constants";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";

interface CredentialDialogProps {
  readonly service: DownstreamService;
  readonly mode: "connect" | "update";
  readonly onSubmit: (credential: string, label?: string) => void;
  readonly onCancel: () => void;
  readonly isPending: boolean;
}

export function CredentialDialog({
  service,
  mode,
  onSubmit,
  onCancel,
  isPending,
}: CredentialDialogProps) {
  const inputConfig = getCredentialInputType(service);
  const [credential, setCredential] = useState("");
  const [label, setLabel] = useState("");

  function handleSubmit(e: React.FormEvent) {
    e.preventDefault();
    const trimmed = credential.trim();
    if (trimmed.length > 0) {
      onSubmit(trimmed, label.trim().length > 0 ? label.trim() : undefined);
    }
  }

  const title =
    mode === "connect"
      ? `Connect to ${service.name}`
      : `Update Credential for ${service.name}`;

  const description =
    mode === "connect"
      ? `Enter your ${inputConfig.label} to connect to this service.`
      : `Enter a new ${inputConfig.label} for this service.`;

  const submitLabel = mode === "connect" ? "Connect" : "Update";

  return (
    <Dialog open onOpenChange={onCancel}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>{title}</DialogTitle>
          <DialogDescription>{description}</DialogDescription>
        </DialogHeader>

        <form onSubmit={handleSubmit} className="space-y-4">
          <div className="space-y-2">
            <Label htmlFor="credential">{inputConfig.label}</Label>
            <Input
              id="credential"
              type="password"
              placeholder={inputConfig.placeholder}
              value={credential}
              onChange={(e) => setCredential(e.target.value)}
              maxLength={8192}
              autoComplete="off"
            />
            {inputConfig.type === "basic" && (
              <p className="text-xs text-muted-foreground">
                Format: username:password
              </p>
            )}
            <p className="text-xs text-muted-foreground">Max 8192 characters</p>
          </div>

          <div className="space-y-2">
            <Label htmlFor="label">Label (optional)</Label>
            <Input
              id="label"
              placeholder="e.g., Production Key"
              value={label}
              onChange={(e) => setLabel(e.target.value)}
              maxLength={200}
            />
          </div>

          <DialogFooter>
            <Button type="button" variant="outline" onClick={onCancel}>
              Cancel
            </Button>
            <Button
              type="submit"
              disabled={credential.trim().length === 0 || isPending}
            >
              {submitLabel}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
