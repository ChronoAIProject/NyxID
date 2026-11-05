import { useEffect, useRef, useState } from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";
import * as Clipboard from "expo-clipboard";
import { Check, Copy } from "lucide-react-native";
import { PrimaryButton } from "../../components/PrimaryButton";
import { loginCodeApi, type IssuedLoginCode } from "../../lib/api/loginCodeApi";
import { LoginCodeMonitor, type LoginCodeMonitorState } from "./loginCodeMonitor";
import { useTheme } from "../../theme/ThemeContext";
import { createDeviceLoginStyles } from "./deviceLoginStyles";
import { fonts } from "../../theme/designTokens";

const COMMAND = "nyxid login --code";

/** m:ss until the code lapses. */
function remaining(expiresAt: string, now: number) {
  const seconds = Math.max(0, Math.ceil((Date.parse(expiresAt) - now) / 1000));
  return `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, "0")}`;
}

export function LoginCodeStatusView({ issued, onClearCode, onNew }: { issued: IssuedLoginCode; onClearCode: () => void; onNew: () => void }) {
  const { colors } = useTheme();
  const styles = createDeviceLoginStyles(colors);
  const [{ status, error, pending }, setMonitorState] = useState<LoginCodeMonitorState>({status: null, error: false, pending: false});
  const monitor = useRef<LoginCodeMonitor | null>(null);
  const [now, setNow] = useState(() => Date.now());
  const codePending = now < Date.parse(issued.expires_at) && (!status || status.status === "pending");
  useEffect(() => { if (!codePending) onClearCode(); }, [codePending, onClearCode]);
  useEffect(() => {
    const current = new LoginCodeMonitor(loginCodeApi, issued.request_id, setMonitorState);
    monitor.current = current;
    current.start();
    const clock = setInterval(() => setNow(Date.now()), 1000);
    return () => { current.stop(); monitor.current = null; clearInterval(clock); };
  }, [issued.request_id]);
  const act = (action: "cancel" | "revoke") => monitor.current?.act(action);
  const [copied, setCopied] = useState(false);
  useEffect(() => {
    if (!copied) return;
    const timer = setTimeout(() => setCopied(false), 1500);
    return () => clearTimeout(timer);
  }, [copied]);
  return <View style={styles.inputSection}>
    <Text style={styles.inputLabel}>{codePending ? "One-time terminal login" : status?.status === "redeemed" ? "Login redeemed" : "Login code closed"}</Text>
    {codePending && <>
      {/* The code centred in its own panel, with Copy, then the command to run. */}
      <View style={[local.ticket, { backgroundColor: colors.card, borderColor: colors.border }]}>
        <Text
          selectable
          accessibilityLabel={`Login code ${issued.code.split("").join(" ")}`}
          style={[local.code, { color: colors.textPrimary }]}
        >
          {issued.code}
        </Text>
        <Pressable
          accessibilityRole="button"
          accessibilityLabel={copied ? "Code copied" : "Copy code"}
          hitSlop={8}
          onPress={() => {
            void Clipboard.setStringAsync(issued.code)
              .then((done) => setCopied(done !== false))
              .catch(() => setCopied(false));
          }}
          style={({ pressed }) => [
            local.copy,
            { borderColor: colors.controlBorder, opacity: pressed ? 0.7 : 1 },
          ]}
        >
          {copied ? (
            <Check size={15} color={colors.success} />
          ) : (
            <Copy size={15} color={colors.textPrimary} />
          )}
          <Text style={[local.copyLabel, { color: colors.textPrimary }]}>
            {copied ? "Copied" : "Copy"}
          </Text>
        </Pressable>
      </View>
      <Text style={[local.meta, { color: colors.textSecondary }]}>
        Expires in {remaining(issued.expires_at, now)} · works once
      </Text>
      <View style={local.command}>
        <Text style={[local.commandLabel, { color: colors.textSecondary }]}>
          In your terminal, run
        </Text>
        <View style={[local.commandChip, { backgroundColor: colors.segmentTrack }]}>
          <Text selectable style={[local.commandText, { color: colors.textPrimary }]}>
            {COMMAND}
          </Text>
        </View>
      </View>
      <PrimaryButton label="Cancel code" kind="ghost" disabled={pending} onPress={() => void act("cancel")} />
    </>}
    {status?.redeemed_at && <>
      <Text style={styles.cautionText}>Device: {status.client_label ?? "Not provided"}</Text>
      <Text style={styles.cautionText}>IP: {status.client_ip ?? "Unavailable"} ({status.client_ip_attribution})</Text>
      <Text style={styles.cautionText}>Redeemed: {new Date(status.redeemed_at).toLocaleString()}</Text>
      <Text style={styles.cautionText}>Access: {status.auth_kind === "agent_key" ? "Restricted Agent Key" : "Full account session"}</Text>
    </>}
    {status?.can_revoke && <PrimaryButton label="Revoke login" kind="danger" disabled={pending} onPress={() => void act("revoke")} />}
    {error && <Text style={styles.fieldErrorText} accessibilityRole="alert">Could not update login status. Try again.</Text>}
    {!codePending && <PrimaryButton label="Generate another code" kind="ghost" onPress={onNew} />}
  </View>;
}

const local = StyleSheet.create({
  ticket: {
    borderRadius: 14,
    borderWidth: 1,
    paddingVertical: 20,
    paddingHorizontal: 16,
    alignItems: "center",
    justifyContent: "center",
    gap: 14,
  },
  code: {
    fontFamily: fonts.mono,
    fontSize: 30,
    lineHeight: 38,
    letterSpacing: 4,
    textAlign: "center",
  },
  copy: {
    flexDirection: "row",
    alignItems: "center",
    gap: 6,
    minHeight: 36,
    paddingHorizontal: 14,
    borderRadius: 18,
    borderWidth: 1,
  },
  copyLabel: { fontFamily: fonts.bodySemi, fontSize: 13, lineHeight: 18 },
  meta: { fontFamily: fonts.body, fontSize: 13, lineHeight: 18, textAlign: "center" },
  command: { alignItems: "center", gap: 6 },
  commandLabel: { fontFamily: fonts.body, fontSize: 13, lineHeight: 18 },
  commandChip: { borderRadius: 8, paddingHorizontal: 12, paddingVertical: 8 },
  // Monospace keeps the double hyphen from joining into a dash.
  commandText: { fontFamily: fonts.mono, fontSize: 14, lineHeight: 20 },
});
