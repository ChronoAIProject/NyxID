import { ServiceIcon } from "../../components/ServiceIcon";
import { useEffect, useRef, useState } from "react";
import { View } from "react-native";
import {
  ActionButton,
  Card,
  Label,
} from "../../components/workspace/WorkspaceUI";
import { useTheme } from "../../theme/ThemeContext";
import type { AssistantHistory } from "./assistantTypes";

type Approval = AssistantHistory["approvals"][number];
export function AssistantApprovalCard({
  approval,
  disabled,
  onDecide,
}: {
  approval: Approval;
  disabled: boolean;
  onDecide: (approved: boolean) => Promise<void>;
}) {
  const { colors } = useTheme();
  const [now, setNow] = useState(Date.now());
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const [decided, setDecided] = useState<boolean>();
  const lock = useRef(false);
  const alive = useRef(true);
  useEffect(() => {
    alive.current = true;
    const timer = setInterval(() => setNow(Date.now()), 30_000);
    return () => {
      alive.current = false;
      clearInterval(timer);
    };
  }, []);
  const remaining = Date.parse(approval.expires_at) - now;
  const expired = !Number.isFinite(remaining) || remaining <= 0;
  async function decide(approved: boolean) {
    if (lock.current || disabled || expired || decided !== undefined) return;
    lock.current = true;
    setBusy(true);
    setError(undefined);
    try {
      await onDecide(approved);
      if (alive.current) setDecided(approved);
    } catch (cause) {
      if (alive.current)
        setError(
          cause instanceof Error
            ? cause.message
            : "Could not send your decision. Try again.",
        );
    } finally {
      lock.current = false;
      if (alive.current) setBusy(false);
    }
  }
  return (
    <Card>
      <View accessibilityLiveRegion="polite">
        <Label style={{ fontWeight: "600", fontSize: 13 }}>
          {decided === true
            ? "Approved and sent"
            : decided === false
              ? "Request denied"
              : "Approval required"}
        </Label>
        {decided === undefined ? (
          <Label muted>
            Review the destination and access scope before this action runs.
          </Label>
        ) : null}
      </View>
      <View style={{ flexDirection: "row", alignItems: "center", gap: 8 }}>
        <ServiceIcon slug={approval.service_slug ?? "custom"} size="sm" />
        <Label style={{ flex: 1 }}>
          {approval.service_name}: {approval.summary}
        </Label>
      </View>
      <View
        style={{
          flexDirection: "row",
          flexWrap: "wrap",
          gap: 6,
          alignItems: "center",
        }}
      >
        <Label muted style={{ fontSize: 10 }}>
          Scope
        </Label>
        {[
          approval.service_slug,
          approval.agent_key_prefix,
          approval.approval_mode === "per_request"
            ? "per-request approval"
            : approval.approval_mode === "grant"
              ? "grant · grant"
              : undefined,
        ]
          .filter(Boolean)
          .map((label) => (
            <View
              key={label}
              style={{
                borderWidth: 1,
                borderColor: colors.border,
                backgroundColor: colors.cardSoft,
                borderRadius: 6,
                paddingHorizontal: 6,
                paddingVertical: 2,
              }}
            >
              <Label muted mono style={{ fontSize: 10 }}>
                {label}
              </Label>
            </View>
          ))}
      </View>
      {approval.approval_mode === "grant" && decided === undefined ? (
        <Label muted>
          Approving also allows repeat {approval.service_slug} writes for the
          next grant without asking again.
        </Label>
      ) : null}
      {decided === undefined ? (
        <>
          <Label muted>
            {expired
              ? "expired"
              : `expires in ${Math.max(1, Math.ceil(remaining / 60_000))} min`}
          </Label>
          <View style={{ flexDirection: "row", flexWrap: "wrap", gap: 8 }}>
            <ActionButton
              label="Approve and send"
              compact
              variant="secondary"
              disabled={disabled || expired || busy}
              loading={busy}
              onPress={() => void decide(true)}
            />
            <ActionButton
              label="Deny"
              compact
              variant="danger"
              disabled={disabled || expired || busy}
              onPress={() => void decide(false)}
            />
          </View>
          <Label muted>Nothing is sent until you decide.</Label>
        </>
      ) : decided === false ? (
        <Label muted>Nothing was sent.</Label>
      ) : null}
      {error ? (
        <Label accessibilityRole="alert" style={{ color: colors.danger }}>
          {error}
        </Label>
      ) : null}
    </Card>
  );
}

/** Same future/past distance copy as web formatTimeDistance. */
export function waitingTimeDistance(value: string, now = Date.now()) {
  const date = Date.parse(value);
  if (!Number.isFinite(date)) return "N/A";
  const diff = date - now,
    seconds = Math.floor(Math.abs(diff) / 1000);
  const prefix = diff > 0 ? "in " : "",
    suffix = diff > 0 ? "" : " ago";
  if (seconds < 60) return diff > 0 ? "in a moment" : "just now";
  if (seconds < 3600) return `${prefix}${Math.floor(seconds / 60)}m${suffix}`;
  if (seconds < 86400)
    return `${prefix}${Math.floor(seconds / 3600)}h${suffix}`;
  if (seconds < 604800)
    return `${prefix}${Math.floor(seconds / 86400)}d${suffix}`;
  return new Date(date).toLocaleDateString();
}
export function AssistantWaitingNote({
  items,
  agentName,
  pendingEvents = 0,
}: {
  items: AssistantHistory["waiting"];
  agentName: string;
  pendingEvents?: number;
}) {
  const { colors } = useTheme();
  const [now, setNow] = useState(Date.now());
  useEffect(() => {
    if (!items.length) return;
    const timer = setInterval(() => setNow(Date.now()), 30_000);
    return () => clearInterval(timer);
  }, [items.length]);
  return (
    <>
      {pendingEvents > 0 ? (
        <Label muted accessibilityLiveRegion="polite" style={{ fontSize: 11 }}>
          {pendingEvents} {pendingEvents === 1 ? "update" : "updates"} waiting
          for {agentName}'s next turn
        </Label>
      ) : null}
      {items.length ? (
        <View
          accessibilityLabel="Waiting"
          accessibilityLiveRegion="polite"
          style={{
            borderWidth: 1,
            borderColor: colors.border,
            borderRadius: 8,
            paddingHorizontal: 12,
            paddingVertical: 8,
            gap: 4,
          }}
        >
          {items.map((item) => (
            <View key={`${item.kind}:${item.since}:${item.title}`}>
              <View
                style={{
                  flexDirection: "row",
                  gap: 8,
                  alignItems: "center",
                  flexWrap: "wrap",
                }}
              >
                <View
                  accessible={false}
                  style={{
                    width: 6,
                    height: 6,
                    borderRadius: 3,
                    backgroundColor: colors.successTone.text,
                  }}
                />
                <Label style={{ fontSize: 12, flex: 1 }}>{item.title}</Label>
                {item.expires_at ? (
                  <Label muted style={{ fontSize: 11 }}>
                    expires {waitingTimeDistance(item.expires_at, now)}
                  </Label>
                ) : null}
              </View>
              {item.detail ? (
                <Label muted style={{ fontSize: 11, paddingLeft: 14 }}>
                  {item.detail}
                </Label>
              ) : null}
            </View>
          ))}
          <Label muted style={{ fontSize: 11 }}>
            {agentName} continues here by itself when this happens; no need to
            reply.
          </Label>
        </View>
      ) : null}
    </>
  );
}
