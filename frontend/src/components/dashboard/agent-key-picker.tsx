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

  // The user's explicit pick. `null` means "no explicit pick yet, use the
  // default derivation below." Avoids an effect-driven `setSelectedId`
  // cascade after `useApiKeys()` resolves (React Compiler flagged the
  // synchronous setState-in-effect pattern as a cascading render).
  const [userOverride, setUserOverride] = useState<string | null>(null);

  // Effective selection: prefer the user's explicit pick, then the
  // localStorage default if it still matches a live key, then the first
  // active key. `null` only when there are zero keys (zero-state branch
  // renders the "Create your first Agent Key →" affordance).
  const selectedId = useMemo<string | null>(() => {
    const first = activeKeys[0];
    if (!first) return null;
    if (userOverride && activeKeys.some((k) => k.id === userOverride)) {
      return userOverride;
    }
    const stored =
      typeof window !== "undefined" ? localStorage.getItem(LS_KEY) : null;
    if (stored && activeKeys.some((k) => k.id === stored)) {
      return stored;
    }
    return first.id;
  }, [activeKeys, userOverride]);

  // Persist the picked id and notify the caller whenever it changes.
  // `preview = key_prefix + ••••••••` so call sites never reach into the
  // raw `key_prefix` field and the redaction is visually explicit.
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
    <Select value={selectedId ?? undefined} onValueChange={setUserOverride}>
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
