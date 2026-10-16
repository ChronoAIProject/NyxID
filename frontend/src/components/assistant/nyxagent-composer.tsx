import { useEffect, useRef, useState, type ComponentProps } from "react";
import { useQuery } from "@tanstack/react-query";
import { toast } from "sonner";
import { UploadComposer } from "./upload-composer";
import { Button } from "@/components/ui/button";
import { ApiError } from "@/lib/api-client";
import { nyxAgentTransport } from "@/lib/assistant/nyxagent-transport";
import { steeringNotice } from "@/lib/assistant/nyxagent-steering";
import { nyxBotApi } from "@/lib/assistant/nyxbot-api";
import { ComposerSettings } from "./composer-settings";
import { useKeys } from "@/hooks/use-keys";
import type { NyxAgentAccessMode, NyxAgentConversation } from "@/schemas/assistant-nyxagent";

type UploadProps = ComponentProps<typeof UploadComposer>;
type Props = Omit<UploadProps, "onSend"> & {
  readonly conversation?: NyxAgentConversation;
  readonly agentOverride?: NyxAgentAccessMode | null;
  readonly onSend: (...args: [...Parameters<UploadProps["onSend"]>, model?: string]) => Promise<void>;
};

/** Key by person + thread: pending guidance must never move with navigation. */
export function NyxAgentComposer({ conversation, agentOverride, ...props }: Props) {
  const [draftModel, setDraftModel] = useState<string>();
  const [connection, setConnection] = useState("");
  const connections = useKeys();
  const availableConnections = (connections.data ?? []).filter((key) => key.is_active &&
    !(key.credential_source?.type === "org" && !key.credential_source.allowed));
  useEffect(() => { setConnection(""); }, [props.ownerUserId, props.draftKey, conversation?.id]);
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

  return <UploadComposer {...props}
    footerControls={<ComposerSettings
      conversationId={conversation?.id}
      model={conversation?.model ?? draftModel ?? "nyxagent/chat"}
      onDraftModelChange={setDraftModel}
      agentOverride={agentOverride}
      active={props.active || props.sending || submitting}
      disabled={props.disabled}
      connectionChoice={{
        value: connection, onChange: setConnection, disabled: connections.isError,
        options: availableConnections.map((key) => ({ id: key.id,
          label: `${key.credential_source?.type === "org" ? `${key.credential_source.org_name} · ` : "Personal · "}${key.label} (${key.slug})`,
        })),
      }}
    />}
    onVoice={props.onVoice ? async (id, preparation) => {
      if (!conversation && draftModel) await nyxBotApi.changeModel(id, draftModel);
      await props.onVoice!(id, preparation);
    } : undefined}
    allowActiveInput={canSteer}
    sending={submitting || (props.sending && !canSteer)}
    activePlaceholder="Guide this running reply…"
    sendLabel={canSteer ? "Send guidance" : "Send message"}
    controls={<>{props.controls}{notice && <div className="mb-2 ml-[30px] text-12 text-muted-foreground" role="status">
      <p>{steeringNotice(notice.code, notice.outcome)}</p>
      {notice.code === "no_active_turn" && notice.text && <Button type="button" size="sm" variant="outline"
        disabled={submitting || props.active}
        onClick={async () => {
          if (busy.current || props.active) return;
          busy.current = true; setSubmitting(true);
          try { await props.onSend(notice.text!); setNotice(undefined); }
          catch { /* The ordinary send path reports its error and retains this action. */ }
          finally { busy.current = false; setSubmitting(false); }
        }}>Send as new message</Button>}
    </div>}</>}
    onSend={async (text, uploads) => {
      if (!props.active) {
        setNotice(undefined);
        // Uploads already created a persisted draft; apply its selection before admission.
        if (!conversation && uploads?.conversationId && draftModel) {
          if (busy.current) throw new Error("Wait for the current request.");
          busy.current = true;
          setSubmitting(true);
          try {
            await nyxBotApi.changeModel(uploads.conversationId, draftModel);
          } catch (error) {
            toast.error(error instanceof Error ? error.message : "Could not change the model.");
            throw error;
          } finally {
            busy.current = false;
            setSubmitting(false);
          }
        }
        await props.onSend(connection ? `/service ${connection}\n${text}` : text, uploads, draftModel);
        setConnection("");
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
