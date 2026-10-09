import { operationPrice, toolPrice } from "@/lib/tools";
import { useApiKeys } from "@/hooks/use-api-keys";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
} from "@/components/ui/dialog";
import { useState } from "react";
import { Link } from "@tanstack/react-router";
import { useTools, useToolTopics } from "@/hooks/use-tools";
import type { ToolOffering } from "@/schemas/tools";
import { PageHeader } from "@/components/shared/page-header";
import { Card, CardContent } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Skeleton } from "@/components/ui/skeleton";
import { ErrorBanner } from "@/components/shared/error-banner";

export function ToolsPage() {
  const { data: tools = [], isLoading, error, refetch } = useTools();
  const { data: topics = [] } = useToolTopics();
  const [search, setSearch] = useState("");
  const [topic, setTopic] = useState("");
  const filtered = tools.filter(
    (tool) =>
      (!topic || tool.topics.includes(topic)) &&
      `${tool.name} ${tool.description ?? ""} ${tool.supplier ?? ""}`
        .toLowerCase()
        .includes(search.toLowerCase()),
  );
  const groups = new Map<string, ToolOffering[]>();
  for (const tool of filtered) {
    const supplier = tool.supplier ?? tool.provider_label;
    groups.set(supplier, [...(groups.get(supplier) ?? []), tool]);
  }
  return (
    <div className="space-y-6">
      <PageHeader
        title="Tools"
        description="NyxID-provided operations for your AIs. Your own connections stay in AI Services."
      />
      <Input
        aria-label="Search tools"
        placeholder="Search tools, providers…"
        value={search}
        onChange={(e) => setSearch(e.target.value)}
      />
      <div className="flex flex-wrap gap-2">
        <Button
          variant={topic === "" ? "secondary" : "ghost"}
          onClick={() => setTopic("")}
        >
          All topics
        </Button>
        {topics
          .filter((t) => tools.some((tool) => tool.topics.includes(t.slug)))
          .map((t) => (
            <Button
              key={t.slug}
              variant={topic === t.slug ? "secondary" : "ghost"}
              onClick={() => setTopic(t.slug)}
            >
              {t.label}
            </Button>
          ))}
      </div>
      {isLoading ? (
        <Skeleton className="h-48" />
      ) : error ? (
        <ErrorBanner message="Unable to load tools" onRetry={refetch} />
      ) : filtered.length === 0 ? (
        <div className="rounded-xl border border-dashed border-border p-8 text-12 text-muted-foreground">
          No tools match. Tools use NyxID-provided access; add your own API
          connections in AI Services.
        </div>
      ) : (
        Array.from(groups, ([supplier, rows]) => (
          <section key={supplier} className="space-y-3">
            <h2 className="text-15 font-semibold">{supplier}</h2>
            <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
              {rows.map((tool) => (
                <ToolCard key={tool.id} tool={tool} />
              ))}
            </div>
          </section>
        ))
      )}
    </div>
  );
}
export function ToolCard({ tool }: { tool: ToolOffering }) {
  const [grantsOpen, setGrantsOpen] = useState(false);
  const operationPriced =
    tool.pricing.platform !== "free" &&
    Boolean(tool.pricing.platform.operations?.length);
  return (
    <Card>
      <CardContent className="space-y-3 p-4">
        <div>
          <h3 className="text-15 font-semibold">{tool.name}</h3>
          <p className="text-11 text-text-tertiary">
            {tool.supplier ?? tool.provider_label}
          </p>
        </div>
        <Badge variant="secondary">Provided by NyxID</Badge>
        <p className="text-12 text-muted-foreground">{tool.description}</p>
        <div className="flex flex-wrap gap-1">
          {tool.topics.map((t) => (
            <Badge key={t} variant="secondary">
              {t}
            </Badge>
          ))}
        </div>
        <p className="text-13 font-semibold">{toolPrice(tool)}</p>
        <p className="text-11 text-text-tertiary">
          {tool.limits
            ? `${tool.limits.rate_limit_per_second} requests / second · Burst ${tool.limits.burst}`
            : "No per-user rate limit"}
        </p>
        <details>
          <summary className="cursor-pointer text-12">
            {tool.operations.length} operations
          </summary>
          <div className="mt-3 space-y-3">
            {tool.operations.map((op) => (
              <div
                key={op.name}
                className="space-y-1 border-t border-border/50 pt-2"
              >
                <p className="text-12 font-medium">{op.name}</p>
                <p className="text-11 text-muted-foreground">
                  {op.description}
                </p>
                <code className="break-all text-11">
                  {op.method} {op.path}
                </code>
                {operationPriced && (
                  <p className="text-11 text-text-tertiary">
                    {operationPrice(tool, op.name)} credits / request
                  </p>
                )}
                <div>
                  {op.risk && (
                    <Badge variant={op.risk === "read" ? "success" : "warning"}>
                      {op.risk === "read" ? "Read-only" : "Changes data"}
                    </Badge>
                  )}
                </div>
              </div>
            ))}
          </div>
        </details>
        <div className="flex flex-wrap gap-2">
          <Button onClick={() => setGrantsOpen(true)}>Use with my AIs</Button>
          <ToolGrantsDialog
            tool={tool}
            open={grantsOpen}
            onOpenChange={setGrantsOpen}
          />
          {tool.access.byok && (
            <Button asChild variant="ghost">
              <Link
                to="/keys"
                search={{
                  tab: "services",
                  action: "add-service",
                  slug: tool.slug,
                }}
              >
                Use my own key
              </Link>
            </Button>
          )}
        </div>
      </CardContent>
    </Card>
  );
}

function ToolGrantsDialog({
  tool,
  open,
  onOpenChange,
}: {
  tool: ToolOffering;
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const { data: keys = [], isLoading } = useApiKeys();
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Grant {tool.name} to an AI</DialogTitle>
          <DialogDescription>
            Choose an agent key to review its grants with this tool selected.
          </DialogDescription>
        </DialogHeader>
        {isLoading ? (
          <Skeleton className="h-24" />
        ) : (
          keys
            .filter((key) => key.is_active)
            .map((key) => (
              <Button variant="secondary" key={key.id} asChild>
                <Link
                  to="/keys/api-key/$keyId"
                  params={{ keyId: key.id }}
                  search={{ grant_service: tool.id }}
                >
                  {key.name}
                </Link>
              </Button>
            ))
        )}
        <Button variant="ghost" asChild>
          <Link
            to="/keys"
            search={{ tab: "nyxid", action: "create-key", service: tool.id }}
          >
            Create an agent key
          </Link>
        </Button>
      </DialogContent>
    </Dialog>
  );
}
