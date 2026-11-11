import { useEffect, useRef, useState } from "react";
import { useUiTranslation, translateUi } from "@/i18n/use-ui-translation";
import { Button } from "@/components/ui/button";
import { AssistantLinkModal } from "@/components/assistant/assistant-link-modals";
import { createAssistantConnectLink } from "@/hooks/use-connect-links";
import { nyxBotApi } from "@/lib/assistant/nyxbot-api";
import type { NextStep, SuggestionSetup } from "@/schemas/assistant-onboarding";

/** Resolve live choices before a chat starts; each provider opens real hosted setup. */
export function NextStepConnect({ step, onStart, onClose }: {
  readonly step: NextStep;
  readonly onStart: (step: NextStep) => void;
  readonly onClose: () => void;
}) {
  useUiTranslation();
  const [setup, setSetup] = useState<SuggestionSetup>();
  const [busy, setBusy] = useState(true);
  const [error, setError] = useState(false);
  const [href, setHref] = useState<string | null>(null);
  const live = useRef(true);
  const started = useRef(false);
  const generation = useRef(0);
  const start = useRef(onStart);
  start.current = onStart;

  async function resolve(slug?: string) {
    if (started.current) return;
    const request = ++generation.current;
    const currentRequest = () => live.current && !started.current && generation.current === request;
    setBusy(true);
    setError(false);
    try {
      const current = await nyxBotApi.suggestionSetup(step.id);
      if (!currentRequest()) return;
      if (current.connected || current.choices.length === 0) {
        started.current = true;
        start.current({ ...step, prompt: current.connected ? current.prompt : step.prompt, connect: false });
        onClose();
        return;
      }
      setSetup(current);
      const choice = current.choices.find((c) => c.slug === slug);
      if (choice) {
        const url = await createAssistantConnectLink({ serviceSlug: choice.slug, personal: false, scopes: choice.scopes });
        if (currentRequest()) setHref(url);
      }
    } catch {
      if (currentRequest()) setError(true);
    } finally {
      if (currentRequest()) setBusy(false);
    }
  }
  useEffect(() => {
    live.current = true;
    void resolve();
    return () => { live.current = false; };
    // A new suggestion mounts a new chooser. Callback identity is not a new request.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  return (
    <div className="space-y-3 px-4 py-3" aria-label={translateUi("{{suggestion}} options", { suggestion: step.title })}>
      <p className="type-secondary text-muted-foreground">
        {busy ? translateUi("Checking your connections…") : translateUi("Choose a provider to connect your account.")}
      </p>
      {error ? <p role="alert" className="type-secondary text-destructive">{translateUi("Could not load connection options. Try again.")}</p> : null}
      <div className="flex flex-wrap gap-2">
        {setup?.choices.map((choice) => (
          <Button key={choice.slug} size="sm" variant="outline" disabled={busy} onClick={() => void resolve(choice.slug)}>
            {translateUi("Connect {{provider}}", { provider: choice.name })}
          </Button>
        ))}
        {error ? <Button size="sm" variant="outline" disabled={busy} onClick={() => void resolve()}>{translateUi("Try again")}</Button> : null}
        <Button size="sm" variant="ghost" onClick={onClose}>{translateUi("Cancel")}</Button>
      </div>
      <AssistantLinkModal href={href} onClose={() => { setHref(null); void resolve(); }} />
    </div>
  );
}
