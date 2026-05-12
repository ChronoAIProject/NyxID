import { useState } from "react";
import { useUpdateApiKey } from "@/hooks/use-api-keys";
import { ApiError } from "@/lib/api-client";
import {
  maskApiKey,
  formatDate,
  formatRelativeTime,
  isPastTimestamp,
} from "@/lib/utils";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Badge } from "@/components/ui/badge";
import { Label } from "@/components/ui/label";
import { Checkbox } from "@/components/ui/checkbox";
import {
  Card,
  CardContent,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { KeyRound, Pencil, Check, X } from "lucide-react";
import { toast } from "sonner";

const AVAILABLE_SCOPES = ["proxy", "read", "write", "admin"];

export function DetailsCard({
  name,
  description,
  keyPrefix,
  scopes,
  isActive,
  createdAt,
  lastUsedAt,
  expiresAt,
  keyId,
}: {
  readonly name: string;
  readonly description: string | null;
  readonly keyPrefix: string;
  readonly scopes: string;
  readonly isActive: boolean;
  readonly createdAt: string;
  readonly lastUsedAt: string | null;
  readonly expiresAt: string | null;
  readonly keyId: string;
}) {
  const [editingName, setEditingName] = useState(false);
  const [editName, setEditName] = useState(name);
  const [editDesc, setEditDesc] = useState(description ?? "");
  const [editingScopes, setEditingScopes] = useState(false);
  const [editScopes, setEditScopes] = useState<readonly string[]>(() =>
    scopes.trim().split(/\s+/).filter(Boolean),
  );
  const updateApiKey = useUpdateApiKey();

  function handleSaveName() {
    if (!editName.trim()) return;
    updateApiKey.mutate(
      {
        keyId,
        name: editName.trim(),
        description: editDesc.trim() || undefined,
      },
      {
        onSuccess: () => {
          toast.success("Key updated");
          setEditingName(false);
        },
        onError: (err) => {
          const message =
            err instanceof ApiError ? err.message : "Failed to update";
          toast.error(message);
        },
      },
    );
  }

  const scopesList = scopes.trim().split(/\s+/).filter(Boolean);

  const isExpired = isPastTimestamp(expiresAt);
  const effectiveStatus: "active" | "expired" | "inactive" = isExpired
    ? "expired"
    : isActive
      ? "active"
      : "inactive";
  const statusLabel =
    effectiveStatus === "active"
      ? "Active"
      : effectiveStatus === "expired"
        ? "Expired"
        : "Inactive";
  const statusVariant: "success" | "secondary" | "destructive" =
    effectiveStatus === "active"
      ? "success"
      : effectiveStatus === "expired"
        ? "destructive"
        : "secondary";

  return (
    <Card>
      <CardHeader className="pb-3">
        <div className="flex items-center gap-2">
          <KeyRound className="h-4 w-4 text-primary" />
          <CardTitle className="text-[15px]">Key Details</CardTitle>
        </div>
      </CardHeader>
      <CardContent className="space-y-4">
        {editingName ? (
          <div className="space-y-2">
            <div className="space-y-1.5">
              <Label>Name</Label>
              <Input
                value={editName}
                onChange={(e) => setEditName(e.target.value)}
              />
            </div>
            <div className="space-y-1.5">
              <Label>Description</Label>
              <Input
                value={editDesc}
                onChange={(e) => setEditDesc(e.target.value)}
                placeholder="Optional description"
              />
            </div>
            <div className="flex justify-end gap-2">
              <Button
                size="icon"
                variant="ghost"
                onClick={() => {
                  setEditName(name);
                  setEditDesc(description ?? "");
                  setEditingName(false);
                }}
              >
                <X className="h-4 w-4" />
              </Button>
              <Button
                size="icon"
                variant="ghost"
                onClick={handleSaveName}
                disabled={updateApiKey.isPending}
              >
                <Check className="h-4 w-4" />
              </Button>
            </div>
          </div>
        ) : (
          <div className="flex items-start justify-between">
            <div>
              <p className="text-[12px] font-medium">{name}</p>
              {description && (
                <p className="text-xs text-muted-foreground">{description}</p>
              )}
            </div>
            <Button
              size="icon"
              variant="ghost"
              onClick={() => setEditingName(true)}
            >
              <Pencil className="h-4 w-4" />
            </Button>
          </div>
        )}

        <div className="grid grid-cols-2 gap-x-4 gap-y-3 text-xs">
          <div className="min-w-0 space-y-1">
            <p className="text-muted-foreground">Prefix</p>
            <code className="block truncate rounded bg-muted px-1.5 py-0.5 font-mono text-foreground">
              {maskApiKey(keyPrefix)}
            </code>
          </div>
          <div className="space-y-1">
            <p className="text-muted-foreground">Status</p>
            <div>
              <Badge variant={statusVariant} className="text-[10px]">
                {statusLabel}
              </Badge>
            </div>
          </div>
          <div className="space-y-1">
            <p className="text-muted-foreground">Created</p>
            <p className="text-foreground">{formatDate(createdAt)}</p>
          </div>
          <div className="space-y-1">
            <p className="text-muted-foreground">Last used</p>
            <p className="text-foreground">{lastUsedAt ? formatRelativeTime(lastUsedAt) : "Never"}</p>
          </div>
          {expiresAt && (
            <div className="col-span-2 space-y-1">
              <p className="text-muted-foreground">Expires</p>
              <p className="text-foreground">{new Date(expiresAt).toLocaleString()}</p>
            </div>
          )}
        </div>

        <div>
          <div className="flex items-center justify-between">
            <span className="text-xs text-muted-foreground">Scopes: </span>
            {!editingScopes && (
              <Button
                size="icon"
                variant="ghost"
                className="h-6 w-6"
                onClick={() => {
                  setEditScopes(scopesList);
                  setEditingScopes(true);
                }}
              >
                <Pencil className="h-3 w-3" />
              </Button>
            )}
          </div>
          {editingScopes ? (
            <div className="mt-1 space-y-2">
              <div className="flex flex-wrap gap-2">
                {AVAILABLE_SCOPES.map((s) => (
                  <div key={s} className="flex items-center gap-1.5">
                    <Checkbox
                      id={`scope-${s}`}
                      checked={editScopes.includes(s)}
                      onCheckedChange={(checked) =>
                        setEditScopes((prev) =>
                          checked ? [...prev, s] : prev.filter((p) => p !== s),
                        )
                      }
                    />
                    <Label htmlFor={`scope-${s}`} className="text-xs">
                      {s}
                    </Label>
                  </div>
                ))}
              </div>
              <div className="flex justify-end gap-2">
                <Button
                  size="icon"
                  variant="ghost"
                  className="h-6 w-6"
                  disabled={updateApiKey.isPending}
                  onClick={() => {
                    updateApiKey.mutate(
                      { keyId, scopes: editScopes.join(" ") },
                      {
                        onSuccess: () => {
                          toast.success("Scopes updated");
                          setEditingScopes(false);
                        },
                        onError: (err) => {
                          const message =
                            err instanceof ApiError
                              ? err.message
                              : "Failed to update";
                          toast.error(message);
                        },
                      },
                    );
                  }}
                >
                  <Check className="h-4 w-4" />
                </Button>
                <Button
                  size="icon"
                  variant="ghost"
                  className="h-6 w-6"
                  onClick={() => {
                    setEditScopes(scopesList);
                    setEditingScopes(false);
                  }}
                >
                  <X className="h-4 w-4" />
                </Button>
              </div>
            </div>
          ) : (
            <div className="mt-1 flex flex-wrap gap-1">
              {scopesList.map((s) => (
                <Badge key={s} variant="secondary" className="text-xs">
                  {s}
                </Badge>
              ))}
            </div>
          )}
        </div>
      </CardContent>
    </Card>
  );
}
