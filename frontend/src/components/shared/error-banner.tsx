import { useState, useCallback } from "react";
import { AlertCircle, Loader2 } from "lucide-react";

interface ErrorBannerProps {
  readonly message: string;
  readonly onRetry?: () => void;
}

export function ErrorBanner({ message, onRetry }: ErrorBannerProps) {
  const [retrying, setRetrying] = useState(false);

  const handleRetry = useCallback(() => {
    if (!onRetry || retrying) return;
    setRetrying(true);
    try {
      onRetry();
    } finally {
      setTimeout(() => setRetrying(false), 1500);
    }
  }, [onRetry, retrying]);

  return (
    <div className="flex items-center gap-3 rounded-xl border border-destructive/15 bg-destructive/[0.04] px-4 py-3">
      <div className="flex h-9 w-9 shrink-0 items-center justify-center rounded-lg bg-destructive/10">
        <AlertCircle className="h-4.5 w-4.5 text-destructive" />
      </div>
      <p className="flex-1 text-[12px] text-destructive">{message}</p>
      {onRetry && (
        <button
          type="button"
          onClick={handleRetry}
          disabled={retrying}
          className="inline-flex items-center justify-center shrink-0 rounded-lg border border-destructive/20 bg-destructive/10 w-[52px] h-[30px] text-[12px] font-medium text-destructive transition-colors duration-200 hover:bg-destructive/15 disabled:opacity-60"
        >
          {retrying ? (
            <Loader2 className="h-3.5 w-3.5 animate-spin" />
          ) : (
            "Retry"
          )}
        </button>
      )}
    </div>
  );
}
