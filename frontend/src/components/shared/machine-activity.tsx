import { useUiTranslation } from "@/i18n/use-ui-translation";
import { translateUi } from "@/i18n/use-ui-translation";
import { getFormattingLocale } from "@/lib/formatting-locale";
import { Label } from "@/components/ui/label";
import { useState } from "react";
import { useMachineActivity } from "@/hooks/use-machine-activity";
import { Button } from "@/components/ui/button";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { AgentAvatar } from "@/components/assistant/nyxbot-agent-avatar";

export function MachineActivity({ nodeId }: { readonly nodeId: string }) {
  useUiTranslation();
  const [agent, setAgent] = useState("");
  const [cursors, setCursors] = useState<string[]>([]);
  const before = cursors.at(-1);
  const query = useMachineActivity(nodeId, agent, before);
  const agents = query.data?.agents ?? [];
  const machineName =
    query.data?.machine_name?.trim() || `Machine ${nodeId.slice(0, 8)}`;
  function selectAgent(value: string) {
    setAgent(value === "all" ? "" : value);
    setCursors([]);
  }
  return (
    <div className="min-h-0 space-y-4 overflow-auto p-5">
      <p className="text-13 text-muted-foreground">{translateUi("Actions, agents and outcomes only. Commands, file paths, page contents and private conversations are not shown.")}</p>
      <div className="space-y-1 text-13">
        <Label htmlFor="machine-activity-agent">{translateUi("Agent")}</Label>
        <Select value={agent || "all"} onValueChange={selectAgent}>
          <SelectTrigger
            id="machine-activity-agent"
            aria-label={translateUi("Filter by agent")}
          >
            <SelectValue placeholder={translateUi("All agents")} />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="all">{translateUi("All agents")}</SelectItem>
            {agents.map((option) => (
              <SelectItem key={option.id} value={option.id}>
                {option.display_name?.trim()
                  ? translateUi("{{display_name}} (@{{name}})", { display_name: option.display_name, name: option.name })
                  : translateUi("@{{name}}", { name: option.name })}
              </SelectItem>
            ))}
            <SelectItem value="unknown">{translateUi("Unknown (older node)")}</SelectItem>
          </SelectContent>
        </Select>
      </div>
      {query.isPending && <p role="status">{translateUi("Loading activity…")}</p>}
      {query.error && <p role="alert">{translateUi("Could not load machine activity.")}</p>}
      {query.data?.entries.length === 0 && (
        <p role="status">{translateUi("No activity for this selection.")}</p>
      )}
      <ol className="space-y-3">
        {query.data?.entries.map((entry) => {
          const namedAgent = agents.find((item) => item.id === entry.agent_id);
          return (
            <li
              key={entry.id}
              className="min-w-0 space-y-1 rounded-lg border border-hairline p-3 text-13"
            >
              <div className="flex flex-wrap justify-between gap-2">
                <span className="font-medium">
                  {entry.action.replaceAll(".", " · ")}
                </span>
                <span>{entry.outcome}</span>
              </div>
              <div className="flex min-w-0 items-center gap-2 text-muted-foreground">
                {namedAgent && <AgentAvatar agent={namedAgent} size="sm" />}
                <span className="min-w-0 break-words">
                  {namedAgent
                    ? namedAgent.display_name?.trim()
                      ? translateUi("{{display_name}} (@{{name}})", { display_name: namedAgent.display_name, name: namedAgent.name })
                      : translateUi("@{{name}}", { name: namedAgent.name })
                    : entry.agent_id
                      ? translateUi("Agent unavailable")
                      : translateUi("Unknown (older node)")}
                </span>
              </div>
              <p className="break-words text-muted-foreground">{machineName}</p>
              <time
                className="text-muted-foreground"
                dateTime={entry.created_at}
              >
                {new Date(entry.created_at).toLocaleString(getFormattingLocale())}
              </time>
              {entry.exit_code !== null && (
                <span className="ml-2">{translateUi("Exit ")}{entry.exit_code}</span>
              )}
              {(namedAgent || !entry.agent_id) && (
                <Button
                  variant="ghost"
                  size="sm"
                  onClick={() => {
                    selectAgent(entry.agent_id ?? "unknown");
                  }}
                >{translateUi("Show this agent")}</Button>
              )}
              <details className="pt-1 text-muted-foreground">
                <summary className="cursor-pointer">{translateUi("Correlation IDs")}</summary>
                <dl className="space-y-1 break-all pt-2">
                  {[
                    ["Machine", nodeId],
                    ["Agent", entry.agent_id],
                    ["Acting person", entry.actor_id],
                    ["Operation", entry.operation_id],
                    ["Activity", entry.activity_id],
                    ["Job", entry.job_id],
                  ]
                    .filter(([, value]) => value)
                    .map(([key, value]) => (
                      <div key={key}>
                        <dt className="font-medium">{key}</dt>
                        <dd>{value}</dd>
                      </div>
                    ))}
                </dl>
              </details>
            </li>
          );
        })}
      </ol>
      <div className="flex gap-2">
        <Button
          disabled={!cursors.length}
          onClick={() => setCursors(cursors.slice(0, -1))}
        >{translateUi("Previous")}</Button>
        <Button
          disabled={!query.data?.next_cursor}
          onClick={() => {
            if (query.data?.next_cursor)
              setCursors([...cursors, query.data.next_cursor]);
          }}
        >{translateUi("Next")}</Button>
      </div>
    </div>
  );
}
