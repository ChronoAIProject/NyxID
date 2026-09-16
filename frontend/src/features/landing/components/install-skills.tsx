import { useEffect, useState } from "react";
import { Check, Copy } from "lucide-react";
import { useTranslation } from "react-i18next";
import { Tabs, TabsList, TabsTrigger } from "@/components/ui/tabs";

export const TERMINAL_INSTALL_COMMAND =
  'bash -c "$(curl -fsSL https://raw.githubusercontent.com/ChronoAIProject/NyxID/main/skills/nyxid/scripts/install.sh)"';
export const AGENT_INSTALL_PROMPT =
  "Install nyx skills from https://github.com/ChronoAIProject/NyxID/blob/main/skills/INSTALL.md";

export function InstallSkills() {
  const { t } = useTranslation();
  const [mode, setMode] = useState("llm");
  const [agent, setAgent] = useState("codex");
  const [feedback, setFeedback] = useState<{ text: string; status: "copied" | "failed" } | null>(null);
  const content = mode === "terminal" ? TERMINAL_INSTALL_COMMAND
    : agent === "codex" ? t("install.codexPrompt") : AGENT_INSTALL_PROMPT;
  const copied = feedback?.text === content && feedback.status === "copied";

  useEffect(() => {
    if (!feedback) return;
    const timer = setTimeout(() => setFeedback(null), 2000);
    return () => clearTimeout(timer);
  }, [feedback]);

  async function handleCopy() {
    try {
      await navigator.clipboard.writeText(content);
      setFeedback({ text: content, status: "copied" });
    } catch {
      setFeedback({ text: content, status: "failed" });
    }
  }

  return (
    <section id="install" className="border-y border-border/60 bg-card/40 px-6 py-20">
      <div className="mx-auto max-w-3xl text-center">
        <h2 className="text-3xl font-bold text-foreground md:text-4xl">{t("install.heading")}</h2>
        <p className="mx-auto mt-4 max-w-2xl text-base leading-relaxed text-muted-foreground">{t("install.subheading")}</p>
        <Tabs value={mode} onValueChange={(value) => { setMode(value); setFeedback(null); }} className="mt-8">
          <TabsList className="mx-auto" aria-label={t("install.modeLabel")}>
            <TabsTrigger value="llm" className="text-muted-foreground">{t("install.llm")}</TabsTrigger>
            <TabsTrigger value="terminal" className="text-muted-foreground">{t("install.terminal")}</TabsTrigger>
          </TabsList>
        </Tabs>
        <div className="mt-5 min-h-8">
          {mode === "llm" ? (
            <label className="flex flex-wrap items-center justify-center gap-2 text-sm text-muted-foreground">
              {t("install.agentLabel")}
              <select value={agent} onChange={(event) => { setAgent(event.target.value); setFeedback(null); }}
                className="h-8 max-w-full rounded-lg border border-border bg-card px-2 text-sm text-foreground">
                <option value="codex">{t("install.codex")}</option>
                <option value="other">{t("install.otherAgents")}</option>
              </select>
            </label>
          ) : <p className="text-sm text-muted-foreground">{t("install.terminalScope")}</p>}
        </div>
        <div className="mt-4 overflow-hidden rounded-lg border border-border bg-black/40 text-left">
          <textarea readOnly value={content} onFocus={(event) => event.currentTarget.select()}
            aria-label={mode === "terminal" ? t("install.commandLabel") : t("install.promptLabel")}
            className="block h-44 w-full min-w-0 resize-none bg-transparent px-4 py-4 font-mono text-sm leading-relaxed text-muted-foreground outline-none sm:h-36" />
          <div className="flex items-center justify-between gap-3 border-t border-border px-4 py-2">
            <span role="status" className="min-w-0 text-xs text-muted-foreground">
              {feedback?.text === content && feedback.status === "failed" ? t("install.copyFailed") : ""}
            </span>
            <button type="button" onClick={() => void handleCopy()}
              className="flex h-8 min-w-24 shrink-0 items-center justify-center gap-2 rounded-lg border border-border px-3 text-sm text-foreground transition-colors hover:bg-white/[0.06]">
              {copied ? <Check className="size-4 text-success" aria-hidden="true" /> : <Copy className="size-4" aria-hidden="true" />}
              {copied ? t("install.copied") : t("install.copy")}
            </button>
          </div>
        </div>
        <p className="mt-5 text-sm leading-relaxed text-muted-foreground">
          {mode === "terminal" ? t("install.terminalHelper") : agent === "codex" ? t("install.codexHelper") : t("install.helper")}
        </p>
      </div>
    </section>
  );
}
