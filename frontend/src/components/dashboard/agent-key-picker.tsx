import { useEffect, useMemo, useState } from "react";
import { Link } from "@tanstack/react-router";
import { useApiKeys } from "@/hooks/use-api-keys";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";

const LS_KEY = "nyxid:last-picked-agent-key";

export interface PickedAgentKey {
  readonly id: string;
  readonly name: string;
  /**
   * Non-secret preview of the selected Agent Key, safe to render in the DOM.
   *
   * NyxID's API key list endpoint returns only the stored `key_prefix`
   * (e.g. `nyxid_ag_ab12`), never the full secret. We append the trailing
   * `••••••••` here so users see that the full key is intentionally redacted
   * — the same convention the Agent Keys table uses. Callers MUST treat this
   * value as display-only: it is NOT a usable bearer token.
   */
  readonly preview: string;
}

interface AgentKeyPickerProps {
  /**
   * Called whenever the picked key changes (including the initial selection
   * once keys load, or `null` while there are zero Agent Keys). Use this to
   * substitute the picker's preview into a curl / code example.
   */
  readonly onSelect?: (key: PickedAgentKey | null) => void;
}

/**
 * Pick a NyxID Agent Key (the `nyxid_ag_…`-prefixed API keys used to
 * authenticate proxy / LLM-gateway requests) and persist the choice across
 * visits via localStorage. Replaces the old `YOUR_NYXID_TOKEN` static
 * placeholder in curl examples with the picked key's non-secret preview.
 *
 * Notes:
 * - Shows a "Create your first Agent Key →" affordance instead of an empty
 *   dropdown when the user has zero active keys, satisfying the zero-keys
 *   acceptance criterion in the B.6 brief.
 * - The picker only ever surfaces `key_prefix` (a server-issued preview),
 *   never the raw secret, so it is safe to interpolate into rendered HTML.
 */
export function AgentKeyPicker({ onSelect }: AgentKeyPickerProps) {
  const { data: apiKeys, isLoading } = useApiKeys();
  const activeKeys = useMemo(
    () => (apiKeys ?? []).filter((k) => k.is_active !== false),
    [apiKeys],
  );

  const [selectedId, setSelectedId] = useState<string | null>(() =>
    localStorage.getItem(LS_KEY),
  );

  // Resolve the initial selection once keys have loaded: prefer the
  // previously-picked id if it still exists, otherwise fall back to the
  // first active key. Falls through to `null` (rendering the zero-keys
  // affordance) when there are no keys yet.
  useEffect(() => {
    if (!activeKeys.length) return;
    const stored = localStorage.getItem(LS_KEY);
    const valid = stored && activeKeys.some((k) => k.id === stored);
    const next = valid ? stored : activeKeys[0].id;
    setSelectedId(next);
  }, [activeKeys]);

  // Persist the picked id and notify the caller whenever it changes.
  // We synthesize a `preview` string (key_prefix + redaction marker) here
  // so call sites never have to reach into the raw `key_prefix` field.
  useEffect(() => {
    if (!selectedId) {
      onSelect?.(null);
      return;
    }
    localStorage.setItem(LS_KEY, selectedId);
    const k = activeKeys.find((x) => x.id === selectedId);
    if (!k) {
      onSelect?.(null);
      return;
    }
    onSelect?.({
      id: k.id,
      name: k.name,
      preview: `${k.key_prefix}••••••••`,
    });
  }, [selectedId, activeKeys, onSelect]);

  if (isLoading) {
    return (
      <p className="text-sm text-muted-foreground">Loading keys…</p>
    );
  }

  if (!activeKeys.length) {
    return (
      <p className="text-sm text-muted-foreground">
        No Agent Keys yet.{" "}
        <Link to="/keys" search={{ tab: "nyxid" }} className="underline">
          Create your first Agent Key →
        </Link>
      </p>
    );
  }

  return (
    <Select value={selectedId ?? undefined} onValueChange={setSelectedId}>
      <SelectTrigger className="w-full max-w-sm">
        <SelectValue placeholder="Select an Agent Key" />
      </SelectTrigger>
      <SelectContent>
        {activeKeys.map((k) => (
          <SelectItem key={k.id} value={k.id}>
            {k.name}
          </SelectItem>
        ))}
      </SelectContent>
    </Select>
  );
}
