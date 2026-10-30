import { useState } from "react";
import { Pressable, StyleSheet, View } from "react-native";
import * as Clipboard from "expo-clipboard";
import { Label, Notice } from "../../components/workspace/WorkspaceUI";
import { useTheme } from "../../theme/ThemeContext";
import { TOUCH_TARGET } from "../../theme/designTokens";
import type { AssistantMessage } from "./assistantTypes";
import {
  diagnosticId,
  formatCapacityWait,
  recoveryReason,
  steeringNotice,
  storedError,
} from "./assistantTurnPresentation";

export function AssistantMessageNotices({
  message,
  onOpenBilling,
}: {
  message: AssistantMessage;
  onOpenBilling: () => void;
}) {
  const { colors } = useTheme();
  const [details, setDetails] = useState(false);
  const [review, setReview] = useState(false);
  const [copied, setCopied] = useState<string>();
  const error =
    message.status === "failed" && message.error_code !== "cancelled";
  const detail = message.error_detail;
  const recovery = message.recovery;
  const failure = message.turn_metadata?.failure_reason;
  const billingCodes = [
    "insufficient_credits",
    "wallet_suspended",
    "plan_entitlement_required",
  ];
  async function copy(text: string, success: string) {
    try {
      await Clipboard.setStringAsync(text);
      setCopied(success);
    } catch {
      setCopied("Copy failed. Select the text above.");
    }
  }
  return (
    <View style={styles.container}>
      {message.steering ? (
        <Label muted>
          Steering ·{" "}
          {steeringNotice(message.steering.code, message.steering.outcome)}
        </Label>
      ) : null}
      {message.error_code === "cancelled" ? (
        <Label
          accessibilityRole="text"
          accessibilityLabel="Turn stopped"
          accessibilityLiveRegion="polite"
          muted
        >
          Stopped
        </Label>
      ) : null}
      {error ? (
        <Notice
          tone="error"
          message={storedError(message.error_code, detail?.capacity_wait_ms)}
        />
      ) : null}
      {error && billingCodes.includes(message.error_code ?? "") ? (
        <QuietButton label="Billing & Usage" onPress={onOpenBilling} />
      ) : null}
      {error && detail ? (
        <>
          <QuietButton
            label="Details"
            expanded={details}
            onPress={() => setDetails((value) => !value)}
          />
          {details ? (
            <View style={[styles.details, { borderColor: colors.border }]}>
              {failure?.source === "nyxid" &&
              failure.code === "guest_model_route_required" ? (
                <Label muted>
                  Send one message to this agent as its owner using its
                  configured provider to record the route for guest replies.
                </Label>
              ) : null}
              <Label selectable muted mono>
                {[
                  detail.attempts != null
                    ? `attempts: ${detail.attempts}`
                    : null,
                  detail.capacity_wait_ms
                    ? `capacity: waited ${formatCapacityWait(detail.capacity_wait_ms)} for capacity`
                    : null,
                  `stage: ${detail.stage}`,
                  failure
                    ? `cause: ${[failure.source, failure.stage, failure.code, failure.http_status != null ? `HTTP ${failure.http_status}` : null, failure.nyxid_error_code != null ? `NyxID ${failure.nyxid_error_code}` : null].filter(Boolean).join(" / ")}`
                    : null,
                  detail.http_status != null
                    ? `http: ${detail.http_status}`
                    : null,
                  message.upstream_error_code
                    ? `upstream: ${message.upstream_error_code}`
                    : null,
                  `turn: ${detail.turn_id}`,
                  detail.response_id ? `response: ${detail.response_id}` : null,
                  detail.upstream_request_id
                    ? `request: ${detail.upstream_request_id}`
                    : null,
                  `continuations: ${detail.continuations}`,
                  `at: ${new Date(detail.occurred_at).toLocaleString()}`,
                ]
                  .filter(Boolean)
                  .join("\n")}
              </Label>
              <QuietButton
                label="Copy diagnostic ID"
                onPress={() =>
                  void copy(diagnosticId(message), "Diagnostic ID copied.")
                }
              />
            </View>
          ) : null}
        </>
      ) : null}
      {error && recovery?.status === "review" ? (
        <>
          <Label muted>{recoveryReason(recovery.reason)}</Label>
          {recovery.text != null ? (
            <>
              <QuietButton
                label="Review request"
                expanded={review}
                onPress={() => setReview((value) => !value)}
              />
              {review ? (
                <View style={[styles.details, { borderColor: colors.border }]}>
                  <Label selectable>{recovery.text}</Label>
                  {recovery.attachments.length ? (
                    <Label muted>
                      Attachments: {recovery.attachments.join(", ")}
                    </Label>
                  ) : null}
                  <QuietButton
                    label="Copy request"
                    onPress={() => void copy(recovery.text!, "Request copied.")}
                  />
                </View>
              ) : null}
            </>
          ) : null}
        </>
      ) : null}
      {copied ? (
        <Label muted accessibilityLiveRegion="polite">
          {copied}
        </Label>
      ) : null}
    </View>
  );
}
export function QuietButton({
  label,
  onPress,
  expanded,
  disabled,
}: {
  label: string;
  onPress: () => void;
  expanded?: boolean;
  disabled?: boolean;
}) {
  const { colors } = useTheme();
  return (
    <Pressable
      accessibilityRole="button"
      accessibilityLabel={label}
      accessibilityState={{ expanded, disabled }}
      disabled={disabled}
      onPress={onPress}
      style={styles.button}
    >
      <Label
        style={{
          color: disabled ? colors.textMuted : colors.textSecondary,
          fontSize: 12,
          textDecorationLine: "underline",
        }}
      >
        {label}
      </Label>
    </Pressable>
  );
}
const styles = StyleSheet.create({
  container: { gap: 4 },
  details: { padding: 10, borderRadius: 8, borderWidth: 1, gap: 4 },
  button: {
    minHeight: TOUCH_TARGET,
    justifyContent: "center",
    alignSelf: "flex-start",
    paddingHorizontal: 3,
  },
});
