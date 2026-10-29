import { ServiceIcon } from "../../components/ServiceIcon";
import { useEffect, useState } from "react";
import { View } from "react-native";
import { SegmentControl } from "../../components/SegmentControl";
import {
  ActionButton,
  Card,
  Field,
  Label,
  Notice,
} from "../../components/workspace/WorkspaceUI";
import { useHostedConnect } from "./useHostedConnect";
import {
  connectFormValues,
  connectNeedsOAuthCredentials,
  connectShowsEndpoint,
} from "./connectLinkState";

export function AssistantHostedConnect({
  token,
  active,
  onBusy,
  onComplete,
}: {
  token: string;
  active: boolean;
  onBusy: (busy: boolean) => void;
  onComplete?: () => void;
}) {
  const flow = useHostedConnect(token, active, onComplete);
  const [credential, setCredential] = useState("");
  const [endpoint, setEndpoint] = useState("");
  const [clientId, setClientId] = useState("");
  const [clientSecret, setClientSecret] = useState("");
  const p = flow.preview;
  useEffect(() => onBusy(flow.busy), [flow.busy, onBusy]);
  useEffect(() => {
    setEndpoint(p?.endpoint_url ?? "");
  }, [p?.endpoint_url]);
  useEffect(() => {
    if (!active) {
      setCredential("");
      setClientSecret("");
    }
  }, [active]);
  async function approve() {
    if (!p || flow.disabled) return;
    try {
      const values = connectFormValues(p, flow.usePlatformKey, {
        credential,
        endpoint_url: endpoint,
        oauth_client_id: clientId,
        oauth_client_secret: clientSecret,
      });
      await flow.approve(values);
    } catch (cause) {
      flow.setError(
        cause instanceof Error
          ? cause.message
          : "The connection request failed.",
      );
    } finally {
      setCredential("");
      setClientSecret("");
    }
  }
  if (!p)
    return (
      <View style={{ gap: 12 }}>
        <Notice
          message={flow.error ?? "Loading connection request…"}
          tone={flow.error ? "error" : "info"}
        />
        {flow.error ? (
          <ActionButton
            label="Retry"
            variant="secondary"
            onPress={() => void flow.reload()}
            disabled={!active || flow.loading}
          />
        ) : null}
      </View>
    );
  if (p.status !== "pending")
    return (
      <Card>
        <ServiceIcon slug={p.service_slug} iconUrl={flow.iconUrl} size="md" />
        <Label accessibilityRole="header">
          {p.status === "completed"
            ? `${p.service_name} connected`
            : p.status === "cancelled"
              ? "Connection cancelled"
              : "Connection request expired"}
        </Label>
        {p.status === "completed" ? <Label>Connection completed</Label> : null}
        <Label accessibilityLiveRegion="polite">
          {p.status === "completed"
            ? "Your agent can now retry the original request."
            : "You may now close this page."}
        </Label>
      </Card>
    );
  return (
    <View style={{ gap: 16 }}>
      <ServiceIcon slug={p.service_slug} iconUrl={flow.iconUrl} size="lg" />
      <Label accessibilityRole="header">
        NyxID wants to connect to your {p.service_name}
      </Label>
      <Label muted>Add your connection details and approve the request.</Label>
      <Card>
        <Label>Requested by</Label>
        <Label muted>{p.requested_by ?? "Your NyxID account"}</Label>
        {p.label ? (
          <>
            <Label>Connection name</Label>
            <Label muted>{p.label}</Label>
          </>
        ) : null}
        {p.scopes.length ? (
          <>
            <Label>Requested permissions</Label>
            <Label muted>{p.scopes.join(", ")}</Label>
          </>
        ) : null}
        <Label muted>Expires {new Date(p.expires_at).toLocaleString()}</Label>
        {p.api_key_instructions ? (
          <Label>{p.api_key_instructions}</Label>
        ) : null}
        {p.api_key_url ? (
          <ActionButton
            label="Credential setup"
            variant="secondary"
            compact
            onPress={() => void flow.openProvider(p.api_key_url!)}
          />
        ) : null}
      </Card>
      {flow.error ? <Notice tone="error" message={flow.error} /> : null}
      {flow.platformAvailable ? (
        <View style={{ gap: 6 }}>
          <Label>Connection</Label>
          <SegmentControl
            segments={[
              { label: "Platform key" },
              { label: "Your credentials" },
            ]}
            activeIndex={flow.usePlatformKey ? 0 : 1}
            onPress={(index) => flow.setPlatformChoice(index === 0)}
            disabled={flow.disabled || !!flow.device || !!flow.oauth}
          />
          <Label muted>
            Platform key uses NyxID's shared connection. Your credentials use
            your own account.
          </Label>
        </View>
      ) : null}
      {!flow.device && !flow.oauth ? (
        <>
          {!flow.usePlatformKey && p.connect_method === "api_key" ? (
            <Field
              label={
                p.auth_key_name.trim() ||
                (p.requires_gateway_url
                  ? "Gateway bearer token"
                  : "API key or token")
              }
              value={credential}
              onChangeText={setCredential}
              secureTextEntry
              autoCorrect={false}
              maxLength={16384}
              editable={!flow.disabled}
            />
          ) : null}
          {!flow.usePlatformKey && connectShowsEndpoint(p) ? (
            <Field
              label="Service URL"
              value={endpoint}
              onChangeText={setEndpoint}
              keyboardType="url"
              autoCorrect={false}
              editable={!flow.disabled}
            />
          ) : null}
          {!flow.usePlatformKey && connectNeedsOAuthCredentials(p) ? (
            <>
              <Field
                label="OAuth client ID"
                value={clientId}
                onChangeText={setClientId}
                maxLength={1024}
                editable={!flow.disabled}
              />
              <Field
                label="OAuth client secret"
                value={clientSecret}
                onChangeText={setClientSecret}
                maxLength={16384}
                secureTextEntry
                autoCorrect={false}
                editable={!flow.disabled}
              />
            </>
          ) : null}
          <ActionButton
            label={
              !flow.usePlatformKey && p.connect_method === "api_key"
                ? "Approve & connect"
                : !flow.usePlatformKey &&
                    (connectShowsEndpoint(p) || connectNeedsOAuthCredentials(p))
                  ? "Approve & continue"
                  : "Approve connection"
            }
            onPress={() => void approve()}
            loading={flow.busy}
            disabled={flow.disabled || flow.loading}
          />
        </>
      ) : null}
      {flow.oauth ? (
        <Card>
          <Label accessibilityLiveRegion="polite">
            Finish authorization in the provider window, then return here.
          </Label>
          <ActionButton
            label="Open provider"
            variant="secondary"
            onPress={() => void flow.openProvider(flow.oauth!.url)}
            disabled={flow.disabled}
          />
          <ActionButton
            label="Check status"
            variant="secondary"
            onPress={() => void flow.check()}
            disabled={flow.disabled}
          />
        </Card>
      ) : null}
      {flow.device ? (
        <Card>
          <Label>
            Enter this code at the provider. NyxID will finish the connection
            after authorization.
          </Label>
          <Label
            selectable
            mono
            accessibilityLabel={`Authorization code: ${flow.device.code}`}
          >
            {flow.device.code}
          </Label>
          <Label muted accessibilityLiveRegion="polite">
            {flow.device.status === "slow_down"
              ? `Provider requested a slower check. Checking again in ${flow.device.interval} seconds.`
              : `Checking automatically every ${flow.device.interval} seconds.`}
          </Label>
          <ActionButton
            label="Open provider"
            variant="secondary"
            onPress={() => void flow.openProvider(flow.device!.url)}
            disabled={flow.disabled}
          />
          {flow.error ? (
            <ActionButton
              label="Get a new code"
              variant="secondary"
              onPress={() => void flow.approve({})}
              disabled={flow.disabled}
            />
          ) : null}
        </Card>
      ) : null}
      <ActionButton
        label="Decline"
        variant="secondary"
        onPress={() => void flow.decline()}
        disabled={flow.disabled}
      />
    </View>
  );
}
