import { useEffect, useRef, useState } from "react";
import { Linking, View } from "react-native";
import { ActionButton, Label } from "../../components/workspace/WorkspaceUI";
import { onboardingApi } from "../../lib/api/onboardingApi";
import { connectLinksApi } from "../../lib/api/connectLinksApi";
import { AssistantHostedConnect } from "../assistant/AssistantHostedConnect";
import { assistantLinkTarget } from "../assistant/assistantLinkTarget";
import { firstPartyFrontendUrls } from "../../lib/env";
import { useGroupAvailability } from "../groups/useGroupRuntime";
import type { NextStep, SuggestionSetup } from "./onboardingContracts";

/** The same live provider choices as web; credentials stay in hosted setup. */
export function NextStepConnect({ step, onStart, onClose }: {
  step: NextStep;
  onStart: (step: NextStep) => void;
  onClose: () => void;
}) {
  const availability = useGroupAvailability();
  const [token, setToken] = useState<string>();
  const [setup, setSetup] = useState<SuggestionSetup>();
  const [busy, setBusy] = useState(true);
  const [opened, setOpened] = useState(false);
  const [error, setError] = useState<string>();
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
    setError(undefined);
    try {
      const current = await onboardingApi.suggestionSetup(step.id);
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
        const link = await connectLinksApi.create(choice.slug, undefined, choice.scopes);
        if (!currentRequest()) return;
        const target = assistantLinkTarget(link.connect_url, firstPartyFrontendUrls());
        if (target?.kind === "connect") setToken(target.token);
        else await Linking.openURL(link.connect_url);
        if (currentRequest()) setOpened(true);
      }
    } catch {
      if (currentRequest()) setError("Could not load connection options. Try again.");
    } finally {
      if (currentRequest()) setBusy(false);
    }
  }
  useEffect(() => {
    live.current = true;
    void resolve();
    return () => { live.current = false; };
    // The chooser is keyed by suggestion, not by callback identity.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
  return (
    <View style={{ padding: 12, gap: 10 }}>
      <Label muted>{token ? "Finish connecting your account below." : busy ? "Checking your connections…" : "Choose a provider to connect your account."}</Label>
      {error ? <Label accessibilityRole="alert">{error}</Label> : null}
      {token ? <AssistantHostedConnect
        key={token} token={token} active={availability.active}
        onBusy={setBusy} onComplete={() => { setToken(undefined); void resolve(); }}
      /> : null}
      {!token && setup?.choices.map((choice) => (
        <ActionButton key={choice.slug} label={`Connect ${choice.name}`} disabled={busy} onPress={() => void resolve(choice.slug)} />
      ))}
      {!token && (opened || error) ? <ActionButton label={error ? "Try again" : "Continue with connected account"} disabled={busy} onPress={() => void resolve()} /> : null}
      <ActionButton label="Cancel" onPress={onClose} />
    </View>
  );
}
