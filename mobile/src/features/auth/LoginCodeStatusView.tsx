import { useEffect, useRef, useState } from "react";
import { Text, View } from "react-native";
import { PrimaryButton } from "../../components/PrimaryButton";
import { loginCodeApi, type IssuedLoginCode } from "../../lib/api/loginCodeApi";
import { LoginCodeMonitor, type LoginCodeMonitorState } from "./loginCodeMonitor";
import { useTheme } from "../../theme/ThemeContext";
import { createDeviceLoginStyles } from "./deviceLoginStyles";

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
  return <View style={styles.inputSection}>
    <Text style={styles.inputLabel}>{codePending ? "One-time terminal login" : status?.status === "redeemed" ? "Login redeemed" : "Login code closed"}</Text>
    {codePending && <>
      <Text selectable style={styles.codeInput}>{issued.code}</Text>
      <Text style={styles.cautionText}>Expires {new Date(issued.expires_at).toLocaleTimeString()}</Text>
      <Text style={styles.cautionText}>Terminal command: nyxid login --code</Text>
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
