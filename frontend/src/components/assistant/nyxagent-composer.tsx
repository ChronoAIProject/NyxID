import { useUiTranslation } from "@/i18n/use-ui-translation";
import { translateUi } from "@/i18n/use-ui-translation";
import { translateErrorMessage } from "@/lib/error-messages";
import { useCallback, useEffect, useLayoutEffect, useRef, useState, type ComponentProps } from "react";
import { useQuery } from "@tanstack/react-query";
import { toast } from "sonner";
import { Link2 } from "lucide-react";
import { UploadComposer } from "./upload-composer";
import { Button } from "@/components/ui/button";
import { ApiError } from "@/lib/api-client";
import { nyxAgentTransport } from "@/lib/assistant/nyxagent-transport";
import { isAdmittedSendError } from "@/lib/assistant/send-admission";
import { autoFocusAllowed } from "@/lib/assistant/composer-focus";
import { steeringNotice } from "@/lib/assistant/nyxagent-steering";
import { useNyxAgentModels } from "@/hooks/use-nyxbot-agents";
import { modelCannotViewImages } from "@/lib/assistant/nyxbot-labels";
import { nyxBotApi } from "@/lib/assistant/nyxbot-api";
import { ComposerSettings } from "./composer-settings";
import { ComposerConnectionChip, ComposerConnectionPicker } from "./composer-connection-picker";
import { DropdownMenuItem } from "@/components/ui/dropdown-menu";
import { composerConnectionUnavailableReason, formatComposerConnectionReason } from "@/lib/assistant/composer-connection";
import { useCurrentKeysReader } from "@/hooks/use-current-keys";
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
  const attachmentTrigger = useRef<HTMLButtonElement>(null);
  const connectionChange = useRef<HTMLButtonElement>(null);
  const pickerOpener = useRef<"attachment" | "change">("attachment");
  const pickerSelected = useRef(false);
  const [now, setNow] = useState(Date.now);
  const connections = useKeys();
  const readCurrentKeys = useCurrentKeysReader();
  const renderedAuthority = readCurrentKeys().authority;
  const [heldConnection, setHeldConnection] = useState<string>();
  const held = useRef<{ id: string; authority: string | undefined } | undefined>(undefined);
  const [checkingSend, setCheckingSend] = useState(false);
  const cancelHeldConnection = useCallback(() => {
    held.current = undefined;
    setHeldConnection(undefined);
  }, []);
  const closeConnectionPicker = useCallback(() => {
    cancelHeldConnection();
    setConnectionPickerOpen(false);
  }, [cancelHeldConnection]);
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
  const connectionsRefreshing = connections.isFetching || connections.isPaused;
  const connectionChecking = connectionsRefreshing || checkingSend;
  const connectionUnavailableReason = connections.isError
    ? translateUi("Could not load connections.")
    : connection ? unavailableReason ? formatComposerConnectionReason(unavailableReason) : null : translateUi("No longer listed");
  // Model setup may await a request before sending. Recheck the latest keys
  // and selection after that await rather than authorizing from its closure.
  const liveSelection = useRef(connectionId);
  useLayoutEffect(() => { liveSelection.current = connectionId; });
  useEffect(() => {
    setConnectionId(undefined); closeConnectionPicker();
  }, [props.ownerUserId, props.draftKey, conversation?.id, closeConnectionPicker]);
  useEffect(() => {
    if (!connectionsRefreshing) setCheckingSend(false);
  }, [connectionsRefreshing, connections.data, connections.isError]);
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
    if (connectionLocked) closeConnectionPicker();
  }, [connectionLocked, closeConnectionPicker]);

  function openConnectionPicker(opener: "attachment" | "change") {
    if (connectionLocked) return;
    pickerOpener.current = opener;
    pickerSelected.current = false;
    setNow(Date.now());
    setConnectionPickerOpen(true);
    if (!readCurrentKeys().isRefreshing) void connections.refetch();
  }

  function focusComposer(fallback: "attachment" | "change") {
    if (autoFocusAllowed()) setConnectionFocusRequest((request) => request + 1);
    else (fallback === "change" ? connectionChange : attachmentTrigger).current?.focus({ preventScroll: true });
  }

  function activateConnection(id: string): "accepted" | "held" | "refused" {
    const current = readCurrentKeys();
    const wallNow = Math.max(now, Date.now());
    setNow(wallNow);
    cancelHeldConnection();
    if (current.authority !== renderedAuthority || current.isError) return "refused";
    if (current.isRefreshing) {
      held.current = { id, authority: current.authority };
      setHeldConnection(id);
      return "held";
    }
    const selected = current.data?.find((key) => key.id === id);
    if (!selected || composerConnectionUnavailableReason(selected, wallNow)) return "refused";
    pickerSelected.current = true;
    setConnectionId(id);
    return "accepted";
  }

  // Query observer notifications can be batched. Settlement always reads the
  // cache again; closing, navigation and another activation cancel the hold.
  useEffect(() => {
    const pending = held.current;
    if (!pending || !connectionPickerOpen || connectionLocked) return;
    const current = readCurrentKeys();
    if (pending.authority !== current.authority) { cancelHeldConnection(); return; }
    if (current.isRefreshing) return;
    if (activateConnection(pending.id) === "accepted") closeConnectionPicker();
  });

  function checkConnection() {
    const current = readCurrentKeys();
    const wallNow = Math.max(now, Date.now());
    const row = current.data?.find((key) => key.id === connectionId);
    const reason = row && composerConnectionUnavailableReason(row, wallNow);
    if (reason && !composerConnectionUnavailableReason(row!, now)) setNow(wallNow);
    if (liveSelection.current !== connectionId || (connectionId &&
      (current.authority !== renderedAuthority || current.isError || current.isRefreshing || !row || reason))) {
      if (current.isRefreshing) setCheckingSend(true);
      const message = "The selected connection is unavailable. Choose another connection or remove it before sending.";
      const title = current.isRefreshing
        ? "Checking the selected connection. Send again when it's ready."
        : message;
      toast.error(translateUi(title), {
        id: "composer-connection-unavailable",
        description: current.isRefreshing ? translateUi("Checking availability…")
          : current.isError ? translateUi("Could not load connections.")
          : reason ? formatComposerConnectionReason(reason) : translateUi("No longer listed"),
      });
      throw new Error(message);
    }
  }

  return <UploadComposer {...props}
    attachmentTriggerRef={attachmentTrigger}
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
    attachmentMenuItems={<DropdownMenuItem disabled={connectionLocked} onSelect={() => openConnectionPicker("attachment")}>
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
      <ComposerConnectionPicker open={connectionPickerOpen && !connectionLocked} onOpenChange={(open) => { if (open) setConnectionPickerOpen(true); else closeConnectionPicker(); }}
        now={now} onCloseAutoFocus={() => focusComposer(pickerSelected.current ? "change" : pickerOpener.current)}
        connections={connections.data ?? []} value={connectionId ?? ""}
        onChange={activateConnection} checkingId={heldConnection} onCancelPending={cancelHeldConnection}
        loading={connections.isPending} error={connections.isError} onRetry={() => void connections.refetch()}
        disabled={connectionLocked} />
      {connectionId && <ComposerConnectionChip connection={connection} unavailableReason={connectionUnavailableReason}
        checking={connectionChecking} announceChecking={checkingSend}
        changeButtonRef={connectionChange} onChange={() => openConnectionPicker("change")}
        disabled={connectionLocked} onRemove={() => { setConnectionId(undefined); focusComposer("attachment"); }} />}
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
          catch (error) {
            if (isAdmittedSendError(error)) {
              setConnectionId(undefined); setNotice(undefined);
            }
          }
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
        try {
          await props.onSend(connectionId ? `/service ${connectionId}\n${text}` : text, uploads, draftModel);
        } catch (error) {
          if (isAdmittedSendError(error)) setConnectionId(undefined);
          throw error; // Preserve admission evidence for the editor and uploads.
        }
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
