import { useEffect, useMemo, useState } from "react";
import { createRoot } from "react-dom/client";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  SkillTemplateCard,
  type SkillTemplateOutline,
} from "@/components/assistant/skill-template-card";
import "@/app.css";

// Local-only preview of NYX template cards. The report is produced by the
// backend's `validate_skill_template_dir` check (real outline parser and the
// real `preview_agent_skill` builder) and served from `public/` while
// developing; nothing here talks to a backend.
interface ReportEntry {
  readonly name: string;
  readonly kind: "skill" | "skillset";
  readonly version: string;
  readonly description: string;
  readonly members: readonly string[];
  readonly closure: readonly string[];
  readonly outline: SkillTemplateOutline | null;
  readonly outline_chars?: number;
  readonly advice?: readonly string[];
  readonly problem?: string;
  readonly instructions_chars?: number;
  readonly returns: {
    readonly preview_agent_skill: unknown;
    readonly preview_agent_skill_outline_only: unknown;
    readonly preview_wire_bytes: number;
  };
}

type Group = "Skillset" | "Workflows" | "Behaviours" | "Capabilities";
const GROUPS: readonly Group[] = ["Skillset", "Workflows", "Behaviours", "Capabilities"];
const CAPABILITY = /(twilio|telnyx|stripe|elevenlabs|notion|google|^records-|^sms-|^payment-|^phone-number-|^voice-agent-|^docs-|^files-)/;
const WORKFLOWS = new Set(["booking-with-availability", "order-with-payment-link"]);

function groupOf(entry: ReportEntry): Group {
  if (entry.kind === "skillset") return "Skillset";
  if (WORKFLOWS.has(entry.name)) return "Workflows";
  return CAPABILITY.test(entry.name) ? "Capabilities" : "Behaviours";
}

type ReturnView = "outline" | "preview" | "outline_only";
const RETURN_VIEWS: ReadonlyArray<{ readonly value: ReturnView; readonly label: string }> = [
  { value: "outline", label: "Parsed outline" },
  { value: "preview", label: "preview_agent_skill" },
  { value: "outline_only", label: "Outline-only page" },
];

function Returns({ entry }: { readonly entry: ReportEntry }) {
  const [view, setView] = useState<ReturnView>("outline");
  const value =
    view === "outline"
      ? entry.outline
      : view === "preview"
        ? entry.returns.preview_agent_skill
        : entry.returns.preview_agent_skill_outline_only;
  return (
    <div className="min-w-0 space-y-2">
      <div className="flex flex-wrap gap-1.5" role="group" aria-label="Return to inspect">
        {RETURN_VIEWS.map((option) => (
          <Button
            key={option.value}
            size="sm"
            variant={view === option.value ? "default" : "ghost"}
            aria-pressed={view === option.value}
            onClick={() => setView(option.value)}
          >
            {option.label}
          </Button>
        ))}
      </div>
      <p className="type-secondary text-text-tertiary">
        {entry.returns.preview_wire_bytes.toLocaleString()} bytes as a tool result (budget 9,200)
        {entry.outline_chars !== undefined ? ` · Overview and Approach ${entry.outline_chars.toLocaleString()} characters` : ""}
        {entry.instructions_chars !== undefined ? ` · master prompt ${entry.instructions_chars.toLocaleString()} of 8,000 characters` : ""}
        {` · ${entry.closure.length} resolved dependencies`}
      </p>
      {entry.advice?.length ? (
        <p className="type-secondary text-warning">Advice: {entry.advice.join("; ")}</p>
      ) : null}
      {entry.problem ? <p className="type-secondary text-destructive">No outline: {entry.problem}</p> : null}
      <pre className="type-code max-h-[28rem] overflow-auto whitespace-pre-wrap break-words rounded-lg border border-border/50 bg-background p-3 text-muted-foreground">
        {JSON.stringify(value, null, 2)}
      </pre>
    </div>
  );
}

export function SkillTemplatePreview() {
  const [entries, setEntries] = useState<readonly ReportEntry[] | null>(null);
  const [error, setError] = useState("");
  const [group, setGroup] = useState<Group | "All">("All");
  const [theme, setTheme] = useState<"dark" | "light">("dark");

  useEffect(() => {
    document.documentElement.classList.remove("theme-dark", "theme-light");
    document.documentElement.classList.add(`theme-${theme}`);
  }, [theme]);

  useEffect(() => {
    fetch("/skill-template-report.json")
      .then((response) => {
        if (!response.ok) throw new Error("missing");
        return response.json() as Promise<ReportEntry[]>;
      })
      .then(setEntries)
      .catch(() => setError("No report found. Run the template check with NYX_SKILL_TEMPLATE_REPORT, then copy it to public/skill-template-report.json."));
  }, []);

  const visible = useMemo(
    () =>
      (entries ?? [])
        .filter((entry) => group === "All" || groupOf(entry) === group)
        .slice()
        .sort((a, b) => GROUPS.indexOf(groupOf(a)) - GROUPS.indexOf(groupOf(b)) || a.name.localeCompare(b.name)),
    [entries, group],
  );

  return (
    <main className="min-h-screen bg-background px-4 py-8 text-foreground sm:px-8 sm:py-12">
      <div className="mx-auto max-w-7xl space-y-8">
        <header className="space-y-3">
          <p className="type-secondary-emphasis uppercase tracking-widest text-muted-foreground">
            NyxID · Local preview · NYX skill templates
          </p>
          <h1 className="type-page-title">Skill cards</h1>
          <p className="type-page-description max-w-3xl text-muted-foreground">
            Each card is rendered from the outline the backend parsed out of the skill, next to the exact
            tool results NyxBot would receive. The skillset is presented as one skill whose dependencies are
            its members.
          </p>
          <div className="flex flex-wrap items-center gap-2">
            {(["All", ...GROUPS] as const).map((option) => (
              <Button
                key={option}
                size="sm"
                variant={group === option ? "default" : "ghost"}
                aria-pressed={group === option}
                onClick={() => setGroup(option)}
              >
                {option}
                {entries ? (
                  <span className="text-text-tertiary">
                    {option === "All" ? entries.length : entries.filter((entry) => groupOf(entry) === option).length}
                  </span>
                ) : null}
              </Button>
            ))}
            <span className="flex-1" />
            <Button size="sm" variant="outline" onClick={() => setTheme(theme === "dark" ? "light" : "dark")}>
              {theme === "dark" ? "Light theme" : "Dark theme"}
            </Button>
          </div>
        </header>

        {error ? <p role="alert" className="type-body text-destructive">{error}</p> : null}
        {!entries && !error ? <p className="type-body text-text-tertiary">Loading report…</p> : null}

        <div className="space-y-10">
          {visible.map((entry) => (
            <article key={entry.name} className="grid gap-4 lg:grid-cols-[600px_minmax(0,1fr)]">
              <div className="space-y-2">
                <div className="flex flex-wrap items-center gap-1.5">
                  <Badge variant="secondary">{groupOf(entry)}</Badge>
                  <span className="type-secondary text-text-tertiary">{entry.description}</span>
                </div>
                <SkillTemplateCard
                  name={entry.name}
                  version={entry.version}
                  kind={entry.kind}
                  members={entry.members}
                  outline={entry.outline}
                  needsAttach={entry.kind === "skillset"}
                />
              </div>
              <Returns entry={entry} />
            </article>
          ))}
        </div>
      </div>
    </main>
  );
}

if (import.meta.env.DEV) {
  const container = document.getElementById("root");
  if (container) {
    const root = import.meta.hot?.data.root ?? createRoot(container);
    if (import.meta.hot) import.meta.hot.data.root = root;
    root.render(<SkillTemplatePreview />);
  }
}
