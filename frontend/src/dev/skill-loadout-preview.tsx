import { StrictMode, useState } from "react";
import { createRoot } from "react-dom/client";
import { appLanguageReady } from "@/i18n";
import { SkillTemplateCard, type SkillTemplateOutline } from "@/components/assistant/skill-template-card";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import examples from "./skill-loadout-examples.json";
import "@/app.css";

interface Example {
  readonly name: string;
  readonly kind: "skill" | "skillset";
  readonly version: string;
  readonly description: string;
  readonly members: readonly string[];
  readonly outline: SkillTemplateOutline;
}

// Historical package excerpts for visual inspection, not a verified Ornn catalog.
const samples = examples as readonly Example[];
const loadout = samples.find((sample) => sample.kind === "skillset")!;

export function SkillLoadoutPreview() {
  const [selectedName, setSelectedName] = useState(
    () => new URLSearchParams(window.location.search).get("skill") ?? loadout.name,
  );
  const selected = samples.find((sample) => sample.name === selectedName) ?? loadout;
  const select = (name: string) => {
    setSelectedName(name);
    const url = new URL(window.location.href);
    url.searchParams.set("skill", name);
    window.history.replaceState(null, "", url);
  };

  return (
    <main className="mx-auto max-w-6xl space-y-6 px-4 py-8 sm:px-8">
      <header className="space-y-3 border-b border-border pb-6">
        <div className="flex flex-wrap items-center gap-3">
          <img src="/nyxid-coloured-icon.svg" alt="NyxID" className="size-8" />
          <h1 className="type-page-title">Skill loadout</h1>
          <Badge variant="secondary">Sample data</Badge>
        </div>
        <p className="type-body text-muted-foreground">
          Explore a Singapore phone front desk and the reusable skills behind it.
          Choose a skill to read its overview, approach, and requirements.
        </p>
      </header>
      <div className="grid items-start gap-8 md:grid-cols-[280px_minmax(0,1fr)]">
        <nav aria-label="Sample skills" className="space-y-5">
          <div className="space-y-2">
            <h2 className="type-group-label text-text-tertiary">Sample loadout · {loadout.members.length} skills</h2>
            <Button
              variant={selected.name === loadout.name ? "secondary" : "ghost"}
              className="h-auto w-full justify-start whitespace-normal break-words py-3 text-left"
              aria-current={selected.name === loadout.name ? "page" : undefined}
              onClick={() => select(loadout.name)}
            >
              {loadout.name}
            </Button>
            {loadout.members.map((name) => (
              <Button
                key={name}
                variant={selected.name === name ? "secondary" : "ghost"}
                className="h-auto w-full justify-start whitespace-normal break-words py-2 text-left"
                aria-current={selected.name === name ? "page" : undefined}
                onClick={() => select(name)}
              >
                {name}
              </Button>
            ))}
          </div>
          <div className="space-y-2 border-t border-border pt-4">
            <h2 className="type-group-label text-text-tertiary">More sample skills</h2>
            {samples.filter((sample) => sample.kind === "skill" && !loadout.members.includes(sample.name)).map((sample) => (
              <Button
                key={sample.name}
                variant={selected.name === sample.name ? "secondary" : "ghost"}
                className="h-auto w-full justify-start whitespace-normal break-words py-2 text-left"
                aria-current={selected.name === sample.name ? "page" : undefined}
                onClick={() => select(sample.name)}
              >
                {sample.name}
              </Button>
            ))}
          </div>
        </nav>
        <div className="min-w-0 space-y-4">
          <SkillTemplateCard
            name={selected.name}
            version={selected.version}
            kind={selected.kind}
            outline={selected.outline}
            members={selected.members}
            author={{ kind: "platform", name: "NyxBot Skills" }}
            actions={false}
          />
          <p className="type-secondary text-text-tertiary">
            These examples are for inspection. They are not attached to your account.
          </p>
        </div>
      </div>
    </main>
  );
}

if (import.meta.env.DEV) {
  void appLanguageReady.then(() => {
    createRoot(document.getElementById("root")!).render(
      <StrictMode><SkillLoadoutPreview /></StrictMode>,
    );
  });
}
