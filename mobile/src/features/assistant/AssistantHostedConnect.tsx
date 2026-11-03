import { useEffect, useState } from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";
import { useQuery } from "@tanstack/react-query";
import { ArrowRight, ExternalLink } from "lucide-react-native";
import { ConnectionIdentityHeading } from "../../components/ConnectionIdentityHeading";
import { getCurrentUserProfileRequest } from "../../lib/api/http";
import { useTheme } from "../../theme/ThemeContext";
import { fonts } from "../../theme/designTokens";
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
      <View style={{ gap: 20 }}>
        <ConnectionIdentityHeading
          serviceSlug={p.service_slug}
          serviceName={p.service_name}
          iconUrl={flow.iconUrl}
          complete={p.status === "completed"}
          title={
            p.status === "completed"
              ? `${p.service_name} connected`
              : p.status === "cancelled"
                ? "Connection cancelled"
                : "Connection request expired"
          }
          subtitle={
            p.status === "completed"
              ? "Your agent can now retry the original request."
              : "You may now close this page."
          }
        />
        <SignedInAs active={active} />
      </View>
    );
  return (
    <View style={{ gap: 20 }}>
      <ConnectionIdentityHeading
        serviceSlug={p.service_slug}
        serviceName={p.service_name}
        iconUrl={flow.iconUrl}
        title={`NyxID wants to connect to your ${p.service_name}`}
        subtitle="Add your connection details and approve the request."
      />
      <RequestDetails
        requestedBy={p.requested_by ?? "Your NyxID account"}
        label={p.label}
        scopes={p.scopes}
        instructions={p.api_key_instructions}
        oauthScopes={
          p.scopes.length > 0 &&
          (p.connect_method === "oauth" || p.connect_method === "device_code")
        }
        onCredentialSetup={
          p.api_key_url
            ? () => void flow.openProvider(p.api_key_url!)
            : undefined
        }
      />
      {flow.error ? <Notice tone="error" message={flow.error} /> : null}
      {flow.platformAvailable && p.assistant_intent_locked ? (
        <Label muted>
          {p.use_platform_key
            ? "This link uses NyxID's platform connection."
            : "This link connects your own account."}
        </Label>
      ) : flow.platformAvailable ? (
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
            trailingIcon={(color) => <ArrowRight size={16} color={color} />}
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
      <QuietAction
        label="Decline"
        onPress={() => void flow.decline()}
        disabled={flow.disabled}
      />
      <SignedInAs active={active} />
    </View>
  );
}

/** The web's bordered request summary: muted labels, values on the right. */
function RequestDetails({
  requestedBy,
  label,
  scopes,
  instructions,
  oauthScopes,
  onCredentialSetup,
}: {
  requestedBy: string;
  label?: string | null;
  scopes: string[];
  instructions?: string | null;
  oauthScopes: boolean;
  onCredentialSetup?: () => void;
}) {
  const { colors } = useTheme();
  const muted = { color: colors.textMuted };
  return (
    <View style={[styles.details, { borderColor: colors.border }]}>
      <DetailRow label="Requested by" value={requestedBy} />
      {label ? <DetailRow label="Connection name" value={label} /> : null}
      {scopes.length ? (
        <DetailRow label="Requested permissions" value={scopes.join(", ")} />
      ) : null}
      {instructions ? (
        <Text style={[styles.note, muted]}>{instructions}</Text>
      ) : null}
      {onCredentialSetup ? (
        <Pressable
          accessibilityRole="link"
          onPress={onCredentialSetup}
          hitSlop={{ top: 12, bottom: 12 }}
          style={styles.setup}
        >
          <Text style={[styles.note, muted]}>Credential setup</Text>
          <ExternalLink size={12} color={colors.textMuted} />
        </Pressable>
      ) : null}
      {oauthScopes ? (
        <Text style={[styles.note, muted]}>
          These additional permissions will be requested on top of the provider
          defaults.
        </Text>
      ) : null}
    </View>
  );
}

function DetailRow({ label, value }: { label: string; value: string }) {
  const { colors } = useTheme();
  return (
    <View
      accessible
      accessibilityLabel={`${label}: ${value}`}
      style={styles.row}
    >
      <Text style={[styles.rowLabel, { color: colors.textMuted }]}>
        {label}
      </Text>
      <Text style={[styles.rowValue, { color: colors.textPrimary }]}>
        {value}
      </Text>
    </View>
  );
}

/** Centred text action, like the web's ghost Decline. */
function QuietAction({
  label,
  onPress,
  disabled,
}: {
  label: string;
  onPress: () => void;
  disabled?: boolean;
}) {
  const { colors } = useTheme();
  return (
    <Pressable
      accessibilityRole="button"
      accessibilityState={{ disabled: !!disabled }}
      disabled={disabled}
      onPress={onPress}
      style={({ pressed }) => [styles.quiet, { opacity: pressed ? 0.6 : 1 }]}
    >
      <Text
        style={[
          styles.quietLabel,
          { color: disabled ? colors.textMuted : colors.textPrimary },
        ]}
      >
        {label}
      </Text>
    </Pressable>
  );
}

function SignedInAs({ active }: { active: boolean }) {
  const { colors } = useTheme();
  const profile = useQuery({
    queryKey: ["account", "profile"],
    queryFn: ({ signal }) => getCurrentUserProfileRequest(signal),
    enabled: active,
    staleTime: 60_000,
    retry: false,
  });
  const email = profile.data?.email;
  if (!email) return null;
  return (
    <Text
      style={[
        styles.signedIn,
        { color: colors.textMuted, borderColor: colors.border },
      ]}
    >
      Signed in as <Text style={{ color: colors.textPrimary }}>{email}</Text>
    </Text>
  );
}

const styles = StyleSheet.create({
  details: {
    borderTopWidth: 1,
    borderBottomWidth: 1,
    paddingVertical: 12,
    gap: 6,
  },
  row: { flexDirection: "row", justifyContent: "space-between", gap: 16 },
  rowLabel: { fontFamily: fonts.bodyRegular, fontSize: 12, lineHeight: 18 },
  rowValue: {
    flexShrink: 1,
    fontFamily: fonts.body,
    fontSize: 12,
    lineHeight: 18,
    textAlign: "right",
  },
  note: { fontFamily: fonts.bodyRegular, fontSize: 12, lineHeight: 18 },
  setup: {
    flexDirection: "row",
    alignItems: "center",
    gap: 6,
    alignSelf: "flex-start",
  },
  quiet: {
    alignSelf: "center",
    minHeight: 44,
    minWidth: 88,
    justifyContent: "center",
    paddingHorizontal: 16,
  },
  quietLabel: { fontFamily: fonts.bodySemi, fontSize: 13, textAlign: "center" },
  signedIn: {
    fontFamily: fonts.bodyRegular,
    fontSize: 12,
    textAlign: "center",
    borderTopWidth: 1,
    paddingTop: 12,
  },
});
