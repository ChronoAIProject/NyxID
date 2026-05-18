import { useState } from "react";
import { useUpdateApiKey } from "@/hooks/use-api-keys";
import { ApiError } from "@/lib/api-client";
import { PLATFORM_OPTIONS } from "@/schemas/agent-bindings";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Monitor, Pencil } from "lucide-react";
import { toast } from "sonner";

const NO_PLATFORM = "__none__";

export function PlatformCard({
  keyId,
  platform,
}: {
  readonly keyId: string;
  readonly platform: string | null;
}) {
  const [editing, setEditing] = useState(false);
  const [selected, setSelected] = useState(platform ?? NO_PLATFORM);
  const updateApiKey = useUpdateApiKey();

  function handleSave() {
    const value = selected === NO_PLATFORM ? null : selected;
    updateApiKey.mutate(
      { keyId, platform: value },
      {
        onSuccess: () => {
          toast.success("Platform updated");
          setEditing(false);
        },
        onError: (err) => {
          const message =
            err instanceof ApiError ? err.message : "Failed to update platform";
          toast.error(message);
        },
      },
    );
  }

  return (
    <Card>
      <CardHeader className="pb-3">
        <div className="flex items-center gap-2">
          <Monitor className="h-4 w-4 text-primary" />
          <CardTitle className="text-[15px]">Platform</CardTitle>
        </div>
        <CardDescription>
          Agent platform using this key
        </CardDescription>
      </CardHeader>
      <CardContent className="space-y-3">
        {editing ? (
          <div className="space-y-2">
            <Select value={selected} onValueChange={setSelected}>
              <SelectTrigger>
                <SelectValue placeholder="Select platform" />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value={NO_PLATFORM}>None</SelectItem>
                {PLATFORM_OPTIONS.map((p) => (
                  <SelectItem key={p} value={p}>
                    {p}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
            <div className="flex justify-end gap-2">
              <Button
                variant="outline"
                onClick={() => {
                  setSelected(platform ?? NO_PLATFORM);
                  setEditing(false);
                }}
              >
                Cancel
              </Button>
              <Button
                variant="primary"
                onClick={handleSave}
                disabled={updateApiKey.isPending || selected === (platform ?? NO_PLATFORM)}
              >
                Save
              </Button>
            </div>
          </div>
        ) : (
          <div className="flex items-center justify-between">
            <Badge variant="secondary">
              {platform ?? "Not set"}
            </Badge>
            <Button
              size="icon"
              variant="ghost"
              className="h-6 w-6 shrink-0"
              onClick={() => setEditing(true)}
            >
              <Pencil className="h-3 w-3" />
            </Button>
          </div>
        )}
      </CardContent>
    </Card>
  );
}
