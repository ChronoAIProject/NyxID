import { useState } from "react";
import { useAdminAuditLog } from "@/hooks/use-admin";
import { formatDateTime } from "@/lib/utils";
import { PageHeader } from "@/components/shared/page-header";
import { Skeleton } from "@/components/ui/skeleton";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Badge } from "@/components/ui/badge";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { ClipboardList, ChevronLeft, ChevronRight, Search } from "lucide-react";

const PER_PAGE = 25;

function responseStatus(entry: { readonly event_data: Record<string, unknown> | null }) {
  const value = entry.event_data?.response_status;
  return typeof value === "number" ? value : null;
}

function statusVariant(
  status: number | null,
): "default" | "secondary" | "destructive" {
  if (status === null) return "secondary";
  if (status >= 500) return "destructive";
  if (status >= 400) return "secondary";
  if (status >= 200) return "default";
  return "secondary";
}

export function AdminAuditLogPage() {
  const [page, setPage] = useState(1);
  const [userIdInput, setUserIdInput] = useState("");
  const [apiKeyIdInput, setApiKeyIdInput] = useState("");
  const [filters, setFilters] = useState<{ userId?: string; apiKeyId?: string }>({});

  const { data, isLoading, error } = useAdminAuditLog(page, PER_PAGE, filters);
  const entries = data?.entries ?? [];
  const total = data?.total ?? 0;
  const totalPages = Math.max(1, Math.ceil(total / PER_PAGE));

  function handleSearch(event: React.FormEvent) {
    event.preventDefault();
    setFilters({
      userId: userIdInput.trim() || undefined,
      apiKeyId: apiKeyIdInput.trim() || undefined,
    });
    setPage(1);
  }

  return (
    <div className="space-y-8">
      <PageHeader
        title="Audit Log"
        description="Filter audit activity by user or agent API key."
      />

      <form onSubmit={handleSearch} className="flex flex-wrap items-center gap-2">
        <div className="relative min-w-[220px] flex-1">
          <Search className="absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-muted-foreground" />
          <Input
            placeholder="Filter by user ID"
            value={userIdInput}
            onChange={(event) => setUserIdInput(event.target.value)}
            className="pl-9"
          />
        </div>
        <div className="relative min-w-[220px] flex-1">
          <Search className="absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-muted-foreground" />
          <Input
            placeholder="Filter by API key ID"
            value={apiKeyIdInput}
            onChange={(event) => setApiKeyIdInput(event.target.value)}
            className="pl-9"
          />
        </div>
        <Button type="submit" variant="outline" size="sm">
          Search
        </Button>
        {(filters.userId || filters.apiKeyId) && (
          <Button
            type="button"
            variant="ghost"
            size="sm"
            onClick={() => {
              setUserIdInput("");
              setApiKeyIdInput("");
              setFilters({});
              setPage(1);
            }}
          >
            Clear
          </Button>
        )}
      </form>

      {isLoading ? (
        <div className="space-y-2">
          {Array.from({ length: 6 }, (_, index) => (
            <Skeleton key={index} className="h-12 w-full" />
          ))}
        </div>
      ) : error ? (
        <div className="flex flex-col items-center justify-center gap-4 py-12 text-center">
          <div className="flex h-14 w-14 items-center justify-center rounded-xl border border-border">
            <ClipboardList className="h-6 w-6 text-muted-foreground" />
          </div>
          <div className="space-y-1">
            <p className="text-[12px] font-medium">Failed to load audit log</p>
            <p className="text-xs text-muted-foreground">Please try again later.</p>
          </div>
        </div>
      ) : entries.length === 0 ? (
        <div className="flex flex-col items-center justify-center gap-4 py-12 text-center">
          <div className="flex h-14 w-14 items-center justify-center rounded-xl border border-border">
            <ClipboardList className="h-6 w-6 text-muted-foreground" />
          </div>
          <div className="space-y-1">
            <p className="text-[12px] font-medium">No audit events found</p>
            <p className="text-xs text-muted-foreground">No audit events match the current filters.</p>
          </div>
        </div>
      ) : (
        <>
          <div className="rounded-xl border border-border">
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>Created</TableHead>
                  <TableHead>Event</TableHead>
                  <TableHead>Agent</TableHead>
                  <TableHead>API Key ID</TableHead>
                  <TableHead>User ID</TableHead>
                  <TableHead>IP</TableHead>
                  <TableHead>User Agent</TableHead>
                  <TableHead>Status</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {entries.map((entry) => {
                  const status = responseStatus(entry);

                  return (
                    <TableRow key={entry.id}>
                      <TableCell className="whitespace-nowrap text-xs text-muted-foreground">
                        {formatDateTime(entry.created_at)}
                      </TableCell>
                      <TableCell className="font-medium">{entry.event_type}</TableCell>
                      <TableCell>
                        {entry.api_key_name ? (
                          <Badge variant="secondary">{entry.api_key_name}</Badge>
                        ) : (
                          <span className="text-xs text-muted-foreground">--</span>
                        )}
                      </TableCell>
                      <TableCell className="font-mono text-xs">
                        {entry.api_key_id ?? "--"}
                      </TableCell>
                      <TableCell className="font-mono text-xs">
                        {entry.user_id ?? "--"}
                      </TableCell>
                      <TableCell className="whitespace-nowrap font-mono text-xs">
                        {entry.ip_address ?? "--"}
                      </TableCell>
                      <TableCell
                        className="max-w-[240px] truncate text-xs text-muted-foreground"
                        title={entry.user_agent ?? undefined}
                      >
                        {entry.user_agent ?? "--"}
                      </TableCell>
                      <TableCell>
                        {status !== null ? (
                          <Badge variant={statusVariant(status)}>{status}</Badge>
                        ) : (
                          <span className="text-xs text-muted-foreground">--</span>
                        )}
                      </TableCell>
                    </TableRow>
                  );
                })}
              </TableBody>
            </Table>
          </div>

          <div className="flex items-center justify-between">
            <p className="text-sm text-muted-foreground">
              Showing {(page - 1) * PER_PAGE + 1}
              {" - "}
              {Math.min(page * PER_PAGE, total)} of {total}
            </p>
            <div className="flex gap-2">
              <Button
                variant="outline"
                size="sm"
                onClick={() => setPage((current) => Math.max(1, current - 1))}
                disabled={page <= 1}
              >
                <ChevronLeft className="h-3 w-3" />
                Previous
              </Button>
              <Button
                variant="outline"
                size="sm"
                onClick={() => setPage((current) => Math.min(totalPages, current + 1))}
                disabled={page >= totalPages}
              >
                Next
                <ChevronRight className="h-3 w-3" />
              </Button>
            </div>
          </div>
        </>
      )}
    </div>
  );
}
