import { useId, useState } from "react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { poolOperationSchema } from "@/schemas/pools";

export interface PoolOperation {
  method: string;
  path: string;
}

/** Draft input is local: only an explicit check updates the query. */
export function PoolOperationCheck({
  operation,
  onChange,
}: {
  operation: PoolOperation | null;
  onChange: (operation: PoolOperation | null) => void;
}) {
  const id = useId();
  const [method, setMethod] = useState(operation?.method ?? "POST");
  const [path, setPath] = useState(operation?.path ?? "");
  const [error, setError] = useState<string>();
  function check() {
    const result = poolOperationSchema.safeParse({ method, path });
    if (!result.success) {
      setError(result.error.issues[0]?.message);
      return;
    }
    setError(undefined);
    onChange(result.data);
  }
  return (
    <div className="space-y-2">
      <p className="text-[12px] text-muted-foreground">
        Check which connections allow a particular request. Enter the path
        relative to each connection’s base URL, for example /chat/completions.
        This does not send a request to the provider.
      </p>
      <div className="flex flex-wrap items-end gap-2">
        <div className="w-24 space-y-1">
          <label htmlFor={`${id}-method`} className="text-[12px]">
            Method
          </label>
          <Select
            value={method}
            onValueChange={(v) => {
              setMethod(v);
              setError(undefined);
            }}
          >
            <SelectTrigger id={`${id}-method`}>
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS"].map(
                (m) => (
                  <SelectItem key={m} value={m}>
                    {m}
                  </SelectItem>
                ),
              )}
            </SelectContent>
          </Select>
        </div>
        <div className="min-w-36 flex-1 space-y-1">
          <label htmlFor={`${id}-path`} className="text-[12px]">
            Operation path
          </label>
          <Input
            id={`${id}-path`}
            value={path}
            placeholder="/chat/completions"
            aria-invalid={Boolean(error)}
            aria-describedby={error ? `${id}-error` : undefined}
            onChange={(e) => {
              setPath(e.target.value);
              setError(undefined);
            }}
            onKeyDown={(e) => {
              if (e.key === "Enter") {
                e.preventDefault();
                check();
              }
            }}
          />
        </div>
        <Button type="button" onClick={check}>
          Check operation
        </Button>
      </div>
      {error && (
        <p
          id={`${id}-error`}
          role="alert"
          className="text-[12px] text-destructive"
        >
          {error}
        </p>
      )}
      {operation && (
        <div className="flex flex-wrap items-center justify-between gap-2 text-[12px]">
          <span className="text-muted-foreground">
            Showing results for{" "}
            <code>
              {operation.method} {operation.path}
            </code>
          </span>
          <Button
            type="button"
            variant="link"
            onClick={() => {
              onChange(null);
              setError(undefined);
            }}
          >
            Clear operation check
          </Button>
        </div>
      )}
    </div>
  );
}
