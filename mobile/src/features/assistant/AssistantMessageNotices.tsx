import { useState, type ReactNode } from "react";
import { Pressable, StyleSheet, View } from "react-native";
import * as Clipboard from "expo-clipboard";
import { CircleAlert } from "lucide-react-native";
import { Label } from "../../components/workspace/WorkspaceUI";
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
        // One compact line instead of a full-width alert box.
        <View
          accessibilityRole="alert"
          style={[styles.error, { backgroundColor: colors.dangerSoftBg }]}
        >
          <CircleAlert size={15} color={colors.danger} />
          <Label
            maxFontSizeMultiplier={1.4}
            style={[styles.errorText, { color: colors.danger }]}
          >
            {storedError(message.error_code, detail?.capacity_wait_ms)}
          </Label>
        </View>
      ) : null}
      {error &&
      (billingCodes.includes(message.error_code ?? "") || detail) ? (
        <View style={styles.actions}>
          {billingCodes.includes(message.error_code ?? "") ? (
            <QuietButton label="Usage & credits" onPress={onOpenBilling} />
          ) : null}
          {detail ? (
            <QuietButton
              label="Details"
              expanded={details}
              onPress={() => setDetails((value) => !value)}
            />
          ) : null}
        </View>
      ) : null}
      {error && detail ? (
        <>
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
/** A small tinted action for in-chat follow-ups; rows of them wrap. */
export function QuietButton({
  label,
  onPress,
  expanded,
  disabled,
  icon,
}: {
  label: string;
  onPress: () => void;
  expanded?: boolean;
  disabled?: boolean;
  icon?: ReactNode;
}) {
  const { colors } = useTheme();
  return (
    <Pressable
      accessibilityRole="button"
      accessibilityLabel={label}
      accessibilityState={{ expanded, disabled }}
      disabled={disabled}
      hitSlop={{ top: 7, bottom: 7 }}
      onPress={onPress}
      style={({ pressed }) => [
        styles.button,
        { backgroundColor: colors.segmentTrack, opacity: pressed ? 0.6 : 1 },
      ]}
    >
      {icon}
      <Label
        maxFontSizeMultiplier={1.3}
        style={{
          color: disabled ? colors.textMuted : colors.textPrimary,
          fontSize: 13,
          lineHeight: 17,
        }}
      >
        {label}
      </Label>
    </Pressable>
  );
}
const styles = StyleSheet.create({
  container: { gap: 6 },
  error: {
    flexDirection: "row",
    alignItems: "flex-start",
    gap: 8,
    borderRadius: 10,
    paddingHorizontal: 10,
    paddingVertical: 8,
  },
  errorText: { flex: 1, fontSize: 14, lineHeight: 19 },
  actions: { flexDirection: "row", flexWrap: "wrap", gap: 8 },
  details: { padding: 10, borderRadius: 8, borderWidth: 1, gap: 4 },
  button: {
    minHeight: TOUCH_TARGET - 14,
    flexDirection: "row",
    alignItems: "center",
    alignSelf: "flex-start",
    gap: 5,
    paddingHorizontal: 11,
    borderRadius: 15,
  },
});
