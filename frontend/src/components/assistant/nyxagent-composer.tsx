import { useUiTranslation } from "@/i18n/use-ui-translation";
import { translateUi } from "@/i18n/use-ui-translation";
import { translateErrorMessage } from "@/lib/error-messages";
import { useEffect, useLayoutEffect, useRef, useState, type ComponentProps } from "react";
import { useQuery } from "@tanstack/react-query";
import { toast } from "sonner";
import { Link2 } from "lucide-react";
import { UploadComposer } from "./upload-composer";
import { Button } from "@/components/ui/button";
import { ApiError } from "@/lib/api-client";
import { nyxAgentTransport } from "@/lib/assistant/nyxagent-transport";
import { steeringNotice } from "@/lib/assistant/nyxagent-steering";
import { useNyxAgentModels } from "@/hooks/use-nyxbot-agents";
import { modelCannotViewImages } from "@/lib/assistant/nyxbot-labels";
import { nyxBotApi } from "@/lib/assistant/nyxbot-api";
import { ComposerSettings } from "./composer-settings";
import { ComposerConnectionChip, ComposerConnectionPicker } from "./composer-connection-picker";
import { DropdownMenuItem } from "@/components/ui/dropdown-menu";
import { composerConnectionUnavailableReason, formatComposerConnectionReason } from "@/lib/assistant/composer-connection";
import { useKeys } from "@/hooks/use-keys";
import type { NyxAgentAccessMode, NyxAgentConversation } from "@/schemas/assistant-nyxagent";

type UploadProps = ComponentProps<typeof UploadComposer>;
type Props = Omit<UploadProps, "onSend"> & {
  readonly conversation?: NyxAgentConversation;
  readonly agentModel?: string;
  readonly agentId?: string;
  readonly agentOverride?: NyxAgentAccessMode | null;
  readonly onVoiceSettings?: () => void;
  readonly onSend: (...args: [...Parameters<UploadProps["onSend"]>, model?: string]) => Promise<void>;
};

/** Key by person + thread: pending guidance must never move with navigation. */
export function NyxAgentComposer({ conversation, agentModel, agentId, agentOverride, onVoiceSettings, ...props }: Props) {
  useUiTranslation();
  const [draftModel, setDraftModel] = useState<string>();
  const models = useNyxAgentModels(true, { conversationId: conversation?.id, agentId });
  const selectedModel = conversation?.model ?? draftModel ?? agentModel ?? "nyxagent/chat";
  const cannotViewImages = modelCannotViewImages(models.data?.find((model) => model.id === selectedModel));
  const [connectionId, setConnectionId] = useState<string>();
  const [connectionPickerOpen, setConnectionPickerOpen] = useState(false);
  const [connectionFocusRequest, setConnectionFocusRequest] = useState(0);
  const [now, setNow] = useState(Date.now);
  const connections = useKeys();
  useEffect(() => {
    const current = Date.now();
    const next = Math.min(...(connections.data ?? [])
      .map((key) => Date.parse(key.expires_at ?? ""))
      .filter((expiry) => Number.isFinite(expiry) && expiry > now));
    if (!Number.isFinite(next)) return;
    const timer = setTimeout(() => setNow(Date.now()), Math.min(Math.max(0, next - current), 2 ** 31 - 1));
    return () => clearTimeout(timer);
  }, [connections.data, now]);
  const connection = connections.data?.find((key) => key.id === connectionId);
  const unavailableReason = connection && composerConnectionUnavailableReason(connection, now);
  const connectionUnavailableReason = connections.isError
    ? translateUi("Could not load connections.")
    : connection ? unavailableReason ? formatComposerConnectionReason(unavailableReason) : null : translateUi("No longer listed");
  // Model setup may await a request before sending. Recheck the latest keys
  // and selection after that await rather than authorizing from its closure.
  const liveSelection = useRef({ connectionId, connection, now, error: connections.isError });
  useLayoutEffect(() => {
    liveSelection.current = { connectionId, connection, now, error: connections.isError };
  });
  useEffect(() => { setConnectionId(undefined); setConnectionPickerOpen(false); }, [props.ownerUserId, props.draftKey, conversation?.id]);
  const capabilities = useQuery({
    queryKey: ["nyxagent-steer-capabilities", props.ownerUserId, conversation?.id],
    queryFn: () => nyxAgentTransport.capabilities(conversation!.id),
    enabled: Boolean(props.ownerUserId && conversation?.id),
    staleTime: 60_000,
    retry: false,
  });
  const [notice, setNotice] = useState<{ code: string | null; outcome?: string; text?: string }>();
  const [submitting, setSubmitting] = useState(false);
  const request = useRef<{ text: string; turnId: string; id: string } | undefined>(undefined);
  const busy = useRef(false);
  const turn = conversation?.active_turn;
  const canSteer = Boolean(props.active && capabilities.data?.steer && turn?.steering_allowed && !turn.stop_requested);
  const connectionLocked = Boolean(props.active || props.sending || submitting || props.disabled);
  useEffect(() => {
    if (connectionLocked) setConnectionPickerOpen(false);
  }, [connectionLocked]);

  function openConnectionPicker() {
    if (connectionLocked) return;
    setNow(Date.now());
    setConnectionPickerOpen(true);
    if (!connections.isFetching) void connections.refetch();
  }

  function focusComposer() {
    setConnectionFocusRequest((request) => request + 1);
  }

  function checkConnection() {
    const live = liveSelection.current;
    const reason = live.connection && composerConnectionUnavailableReason(live.connection, live.now);
    if (live.connectionId !== connectionId || (connectionId &&
      (live.error || !live.connection || reason))) {
      const message = "The selected connection is unavailable. Choose another connection or remove it before sending.";
      toast.error(translateErrorMessage(message), {
        description: live.error ? translateUi("Could not load connections.")
          : reason ? formatComposerConnectionReason(reason) : translateUi("No longer listed"),
      });
      throw new Error(message);
    }
  }

  return <UploadComposer {...props}
    focusRequest={(props.focusRequest ?? 0) + connectionFocusRequest}
    beforeSend={() => {
      if (!props.active) {
        try { checkConnection(); } catch { return false; }
      }
      return props.beforeSend?.() ?? true;
    }}
    imageWarning={cannotViewImages ? translateUi("This model can’t view images. You can still send, choose another model, or describe the image in text.") : undefined}
    onVoiceSettings={onVoiceSettings}
    footerControls={<ComposerSettings
      conversationId={conversation?.id}
      agentId={agentId}
      model={selectedModel}
      onDraftModelChange={setDraftModel}
      agentOverride={agentOverride}
      active={props.active || props.sending || submitting}
      disabled={props.disabled}
      onVoiceSettings={onVoiceSettings}
    />}
    attachmentMenuItems={<DropdownMenuItem disabled={connectionLocked} onSelect={openConnectionPicker}>
      <Link2 aria-hidden className="size-4" />{translateUi("Use a connection")}</DropdownMenuItem>}
    onVoice={props.onVoice && !connectionId ? async (id, preparation) => {
      if (!conversation && draftModel) await nyxBotApi.changeModel(id, draftModel);
      await props.onVoice!(id, preparation);
    } : undefined}
    allowActiveInput={canSteer}
    sending={submitting || (props.sending && !canSteer)}
    activePlaceholder={translateUi("Guide this running reply…")}
    sendLabel={canSteer ? translateUi("Send guidance") : translateUi("Send message")}
    controls={<>{props.controls}
      <ComposerConnectionPicker open={connectionPickerOpen && !connectionLocked} onOpenChange={setConnectionPickerOpen}
        now={now} onCloseAutoFocus={focusComposer}
        connections={connections.data ?? []} value={connectionId ?? ""}
        onChange={(id) => {
          const selected = connections.data?.find((key) => key.id === id);
          if (selected && !connections.isError && !composerConnectionUnavailableReason(selected, now)) setConnectionId(id);
        }}
        loading={connections.isPending} error={connections.isError} onRetry={() => void connections.refetch()}
        disabled={connectionLocked} />
      {connectionId && <ComposerConnectionChip connection={connection} unavailableReason={connectionUnavailableReason}
        onChange={openConnectionPicker}
        disabled={connectionLocked} onRemove={() => { setConnectionId(undefined); focusComposer(); }} />}
      {notice && <div className="mb-2 ml-[30px] text-14 text-muted-foreground" role="status">
      <p>{translateUi(steeringNotice(notice.code, notice.outcome))}</p>
      {notice.code === "no_active_turn" && notice.text && <Button type="button" size="sm" variant="outline"
        disabled={submitting || props.active}
        onClick={async () => {
          if (busy.current || props.active) return;
          busy.current = true; setSubmitting(true);
          try {
            checkConnection();
            await props.onSend(connectionId ? `/service ${connectionId}\n${notice.text!}` : notice.text!);
            setConnectionId(undefined); setNotice(undefined);
          }
          catch { /* The ordinary send path reports its error and retains this action. */ }
          finally { busy.current = false; setSubmitting(false); }
        }}>{translateUi("Send as new message")}</Button>}
    </div>}</>}
    onSend={async (text, uploads) => {
      if (!props.active) {
        checkConnection();
        setNotice(undefined);
        // Uploads already created a persisted draft; apply its selection before admission.
        if (!conversation && uploads?.conversationId && draftModel) {
          if (busy.current) throw new Error("Wait for the current request.");
          busy.current = true;
          setSubmitting(true);
          try {
            await nyxBotApi.changeModel(uploads.conversationId, draftModel);
          } catch (error) {
            toast.error(translateErrorMessage(error instanceof Error ? error.message : "Could not change the model."));
            throw error;
          } finally {
            busy.current = false;
            setSubmitting(false);
          }
        }
        checkConnection();
        await props.onSend(connectionId ? `/service ${connectionId}\n${text}` : text, uploads, draftModel);
        setConnectionId(undefined);
        return;
      }
      if (!canSteer || !conversation || !turn || busy.current) throw new Error("Wait for the current request.");
      if (uploads?.attachmentIds.length || [...text].length > (capabilities.data?.steer?.max_chars ?? 32_000)) {
        setNotice({ code: "invalid_request" }); throw new Error("Guidance must be bounded text only.");
      }
      if (request.current?.text !== text || request.current.turnId !== turn.turn_id) {
        request.current = { text, turnId: turn.turn_id, id: crypto.randomUUID() };
      }
      busy.current = true; setSubmitting(true); setNotice(undefined);
      try {
        const receipt = await nyxAgentTransport.steer(conversation.id, text, turn.turn_id, request.current.id);
        setNotice({ code: receipt.code, outcome: receipt.outcome });
        request.current = undefined;
      } catch (error) {
        const code = error instanceof ApiError ? error.errorResponse.error : "steer_unavailable";
        setNotice({ code, text });
        if (code === "no_active_turn") { request.current = undefined; return; }
        if (code === "steer_unavailable" || !(error instanceof ApiError) || error.status >= 500) {
          // Clear the draft on uncertainty. No automatic resend or fresh key.
          return;
        }
        if (code !== "starting") request.current = undefined;
        throw error; // Definite refusal: retain the editable draft.
      } finally { busy.current = false; setSubmitting(false); }
    }} />;
}
