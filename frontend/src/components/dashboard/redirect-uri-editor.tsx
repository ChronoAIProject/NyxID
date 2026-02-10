import { useState } from "react";
import { useUpdateRedirectUris } from "@/hooks/use-services";
import { redirectUriSchema } from "@/schemas/services";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Plus, Trash2 } from "lucide-react";
import { toast } from "sonner";

interface RedirectUriEditorProps {
  readonly serviceId: string;
  readonly initialUris: readonly string[];
}

export function RedirectUriEditor({
  serviceId,
  initialUris,
}: RedirectUriEditorProps) {
  const [uris, setUris] = useState<readonly string[]>(initialUris);
  const [newUri, setNewUri] = useState("");
  const [validationError, setValidationError] = useState<string | null>(null);
  const updateMutation = useUpdateRedirectUris();

  const hasChanges =
    uris.length !== initialUris.length ||
    uris.some((uri, i) => uri !== initialUris[i]);

  function handleAdd() {
    const result = redirectUriSchema.safeParse(newUri);
    if (!result.success) {
      setValidationError(result.error.issues[0]?.message ?? "Invalid URI");
      return;
    }

    if (uris.includes(newUri)) {
      setValidationError("This URI is already in the list");
      return;
    }

    setUris([...uris, newUri]);
    setNewUri("");
    setValidationError(null);
  }

  function handleRemove(index: number) {
    setUris(uris.filter((_, i) => i !== index));
  }

  async function handleSave() {
    try {
      await updateMutation.mutateAsync({
        serviceId,
        redirectUris: uris,
      });
      toast.success("Redirect URIs updated");
    } catch {
      toast.error("Failed to update redirect URIs");
    }
  }

  return (
    <div className="space-y-3">
      <div className="space-y-2">
        {uris.map((uri, index) => (
          <div
            key={uri}
            className="flex items-center gap-2"
          >
            <code className="flex-1 truncate rounded bg-muted px-2 py-1 text-xs">
              {uri}
            </code>
            <Button
              variant="ghost"
              size="icon"
              className="h-7 w-7 shrink-0 text-muted-foreground hover:text-destructive"
              onClick={() => handleRemove(index)}
            >
              <Trash2 className="h-3 w-3" />
              <span className="sr-only">Remove URI</span>
            </Button>
          </div>
        ))}
        {uris.length === 0 && (
          <p className="text-xs text-muted-foreground">
            No redirect URIs configured.
          </p>
        )}
      </div>

      <div className="flex items-start gap-2">
        <div className="flex-1 space-y-1">
          <Input
            placeholder="https://example.com/callback"
            value={newUri}
            onChange={(e) => {
              setNewUri(e.target.value);
              setValidationError(null);
            }}
            onKeyDown={(e) => {
              if (e.key === "Enter") {
                e.preventDefault();
                handleAdd();
              }
            }}
            className="h-8 text-sm"
          />
          {validationError && (
            <p className="text-xs text-destructive">{validationError}</p>
          )}
        </div>
        <Button
          variant="outline"
          size="sm"
          className="h-8 shrink-0"
          onClick={handleAdd}
        >
          <Plus className="mr-1 h-3 w-3" />
          Add
        </Button>
      </div>

      {hasChanges && (
        <Button
          size="sm"
          onClick={() => void handleSave()}
          isLoading={updateMutation.isPending}
        >
          Save redirect URIs
        </Button>
      )}
    </div>
  );
}
