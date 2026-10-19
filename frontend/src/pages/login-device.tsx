import { LoginAgentKeyPage } from "./login-agent-key";
export { LoginDeviceShell, ApprovalCaution, PreviewPanel } from "@/components/auth/login-request-preview";

export function LoginDevicePage() {
  return <LoginAgentKeyPage flow="device" />;
}
