import { useState } from "react";
import type { ProviderConfig } from "@/types/api";
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
import { ExternalLink } from "lucide-react";

interface ApiKeyDialogProps {
  readonly provider: ProviderConfig;
  readonly onSubmit: (apiKey: string, label?: string) => void;
  readonly onCancel: () => void;
  readonly isPending: boolean;
}

export function ApiKeyDialog({
  provider,
  onSubmit,
  onCancel,
  isPending,
}: ApiKeyDialogProps) {
  const [apiKey, setApiKey] = useState("");
  const [label, setLabel] = useState("");

  function handleSubmit(e: React.FormEvent) {
    e.preventDefault();
    const trimmedKey = apiKey.trim();
    if (trimmedKey.length > 0) {
      onSubmit(
        trimmedKey,
        label.trim().length > 0 ? label.trim() : undefined,
      );
    }
  }

  return (
    <Dialog open onOpenChange={onCancel}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Connect to {provider.name}</DialogTitle>
          <DialogDescription>
            Enter your API key to connect this provider. Your key will be
            encrypted at rest.
          </DialogDescription>
        </DialogHeader>

        <form onSubmit={handleSubmit} className="space-y-4">
          {provider.api_key_instructions && (
            <div className="rounded-md bg-muted p-3 text-sm text-muted-foreground">
              {provider.api_key_instructions}
            </div>
          )}

          {provider.api_key_url && (
            <a
              href={provider.api_key_url}
              target="_blank"
              rel="noopener noreferrer"
              className="inline-flex items-center gap-1.5 text-sm text-primary hover:underline"
            >
              Get your API key
              <ExternalLink className="h-3 w-3" />
            </a>
          )}

          <div className="space-y-2">
            <Label htmlFor="provider-api-key">API Key</Label>
            <Input
              id="provider-api-key"
              type="password"
              placeholder="sk-..."
              value={apiKey}
              onChange={(e) => setApiKey(e.target.value)}
              maxLength={4096}
              autoComplete="off"
            />
            <p className="text-xs text-muted-foreground">
              Max 4096 characters
            </p>
          </div>

          <div className="space-y-2">
            <Label htmlFor="provider-label">Label (optional)</Label>
            <Input
              id="provider-label"
              placeholder="e.g., Production Key"
              value={label}
              onChange={(e) => setLabel(e.target.value)}
              maxLength={200}
            />
          </div>

          {isPending && (
            <p className="text-xs text-muted-foreground">
              Validating your API key with {provider.name}...
            </p>
          )}

          <DialogFooter>
            <Button
              type="button"
              variant="outline"
              onClick={onCancel}
              disabled={isPending}
            >
              Cancel
            </Button>
            <Button
              type="submit"
              disabled={apiKey.trim().length === 0 || isPending}
              isLoading={isPending}
            >
              {isPending ? "Validating..." : "Connect"}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
