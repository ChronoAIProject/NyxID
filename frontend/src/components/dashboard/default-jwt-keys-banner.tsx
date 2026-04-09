import { useState } from "react";
import { usePublicConfig } from "@/hooks/use-public-config";
import { ShieldAlert, X } from "lucide-react";

export function DefaultJwtKeysBanner() {
  const { data: config } = usePublicConfig();
  const [dismissed, setDismissed] = useState(false);

  if (!config?.using_default_jwt_keys || dismissed) return null;

  return (
    <div className="flex items-center gap-3 border-b border-amber-500/30 bg-amber-500/10 px-4 py-2 text-sm text-amber-700 dark:text-amber-400">
      <ShieldAlert className="h-4 w-4 shrink-0" />
      <span>
        This instance is using the default JWT signing keys. Generate your own
        before exposing it publicly:{" "}
        <code className="rounded bg-amber-500/20 px-1 py-0.5 text-xs font-mono">
          openssl genrsa -out keys/private.pem 4096
        </code>
      </span>
      <button
        onClick={() => setDismissed(true)}
        className="ml-auto shrink-0 rounded p-0.5 hover:bg-amber-500/20"
        aria-label="Dismiss"
      >
        <X className="h-3.5 w-3.5" />
      </button>
    </div>
  );
}
