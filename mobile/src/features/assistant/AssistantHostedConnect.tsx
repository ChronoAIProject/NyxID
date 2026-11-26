import { localizedError } from "../../i18n/localizedError";
import { Translation } from "../../i18n/Translation";
import { useAppLanguage } from "../../i18n/LanguageProvider";
import { translateApp } from "../../i18n/appTranslation";
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
  useAppLanguage();
  const flow = useHostedConnect(token, active, onComplete);
  const [credential, setCredential] = useState("");
  const [fieldValues, setFieldValues] = useState<Record<string, string>>({});
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
      setFieldValues({});
      setClientSecret("");
    }
  }, [active]);
  async function approve() {
    if (!p || flow.disabled) return;
    try {
      let composed = credential;
      if (
        !flow.usePlatformKey &&
        p.connect_method === "api_key" &&
        flow.credentialFields.length
      ) {
        const fields: Record<string, string> = {};
        for (const field of flow.credentialFields) {
          const value = fieldValues[field.name]?.trim();
          if (!value) throw localizedError("Enter {{label}}.", { label: field.label});
          fields[field.name] = value;
        }
        composed = JSON.stringify(fields);
      }
      const values = connectFormValues(p, flow.usePlatformKey, {
        credential: composed,
        endpoint_url: endpoint,
        oauth_client_id: clientId,
        oauth_client_secret: clientSecret,
      });
      await flow.approve(values);
    } catch (cause) {
      flow.setError(() => (
        cause instanceof Error
          ? cause.message
          : translateApp("The connection request failed.")
        ),
      );
    } finally {
      setCredential("");
      setFieldValues({});
      setClientSecret("");
    }
  }
  if (!p)
    return (
      <View style={{ gap: 12 }}>
        <Notice
          message={flow.error ?? translateApp("Loading connection request…")}
          tone={flow.error ? "error" : "info"}
        />
        {flow.error ? (
          <ActionButton
            label={translateApp("Retry")}
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
              ? translateApp("{{service}} connected", { service: p.service_name })
              : p.status === "cancelled"
                ? translateApp("Connection cancelled")
                : translateApp("Connection request expired")
          }
          subtitle={
            p.status === "completed"
              ? translateApp("Your agent can now retry the original request.")
              : translateApp("You may now close this page.")
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
        title={translateApp("NyxID wants to connect to your {{service}}", {
          service: p.service_name,
        })}
        subtitle={translateApp(
          "Add your connection details and approve the request.",
        )}
      />
      <RequestDetails
        requestedBy={p.requested_by ?? translateApp("Your NyxID account")}
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
            ? translateApp("This link uses NyxID's platform connection.")
            : translateApp("This link connects your own account.")}
        </Label>
      ) : flow.platformAvailable ? (
        <View style={{ gap: 6 }}>
          <Label>{translateApp("Connection")}</Label>
          <SegmentControl
            segments={[
              { label: translateApp("Platform key") },
              { label: translateApp("Your credentials") },
            ]}
            activeIndex={flow.usePlatformKey ? 0 : 1}
            onPress={(index) => flow.setPlatformChoice(index === 0)}
            disabled={flow.disabled || !!flow.device || !!flow.oauth}
          />
          <Label muted>
            {translateApp(
              "Platform key uses NyxID's shared connection. Your credentials use your own account.",
            )}
          </Label>
        </View>
      ) : null}
      {!flow.device && !flow.oauth ? (
        <>
          {!flow.usePlatformKey &&
          p.connect_method === "api_key" &&
          flow.credentialFields.length ? (
            flow.credentialFields.map((field) => (
              <Field
                key={field.name}
                label={field.label}
                value={fieldValues[field.name] ?? ""}
                onChangeText={(value) =>
                  setFieldValues((current) => ({ ...current, [field.name]: value }))
                }
                secureTextEntry={field.secret}
                autoCorrect={false}
                maxLength={16384}
                editable={!flow.disabled}
              />
            ))
          ) : !flow.usePlatformKey && p.connect_method === "api_key" ? (
            <Field
              label={
                p.auth_key_name.trim() ||
                (p.requires_gateway_url
                  ? translateApp("Gateway bearer token")
                  : translateApp("API key or token"))
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
              label={translateApp("Service URL")}
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
                label={translateApp("OAuth client ID")}
                value={clientId}
                onChangeText={setClientId}
                maxLength={1024}
                editable={!flow.disabled}
              />
              <Field
                label={translateApp("OAuth client secret")}
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
                ? translateApp("Approve & connect")
                : !flow.usePlatformKey &&
                    (connectShowsEndpoint(p) || connectNeedsOAuthCredentials(p))
                  ? translateApp("Approve & continue")
                  : translateApp("Approve connection")
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
            {translateApp(
              "Finish authorization in the provider window, then return here.",
            )}
          </Label>
          <ActionButton
            label={translateApp("Open provider")}
            variant="secondary"
            onPress={() => void flow.openProvider(flow.oauth!.url)}
            disabled={flow.disabled}
          />
          <ActionButton
            label={translateApp("Check status")}
            variant="secondary"
            onPress={() => void flow.check()}
            disabled={flow.disabled}
          />
        </Card>
      ) : null}
      {flow.device ? (
        <Card>
          <Label>
            {translateApp(
              "Enter this code at the provider. NyxID will finish the connection after authorization.",
            )}
          </Label>
          <Label
            selectable
            mono
            accessibilityLabel={translateApp("Authorization code: {{code}}", {
              code: flow.device.code,
            })}
          >
            {flow.device.code}
          </Label>
          <Label muted accessibilityLiveRegion="polite">
            {flow.device.status === "slow_down"
              ? translateApp(
                  "Provider requested a slower check. Checking again in {{seconds}} seconds.",
                  { seconds: flow.device.interval},
                )
              : translateApp(
                  "Checking automatically every {{seconds}} seconds.",
                  { seconds: flow.device.interval},
                )}
          </Label>
          <ActionButton
            label={translateApp("Open provider")}
            variant="secondary"
            onPress={() => void flow.openProvider(flow.device!.url)}
            disabled={flow.disabled}
          />
          {flow.error ? (
            <ActionButton
              label={translateApp("Get a new code")}
              variant="secondary"
              onPress={() => void flow.approve({})}
              disabled={flow.disabled}
            />
          ) : null}
        </Card>
      ) : null}
      <QuietAction
        label={translateApp("Decline")}
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
  useAppLanguage();
  const { colors } = useTheme();
  const muted = { color: colors.textMuted };
  return (
    <View style={[styles.details, { borderColor: colors.border }]}>
      <DetailRow label={translateApp("Requested by")} value={requestedBy} />
      {label ? (
        <DetailRow label={translateApp("Connection name")} value={label} />
      ) : null}
      {scopes.length ? (
        <DetailRow label={translateApp("Requested permissions")}
          value={scopes.join(", ")} />
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
          <Text style={[styles.note, muted]}>
            {translateApp("Credential setup")}
          </Text>
          <ExternalLink size={12} color={colors.textMuted} />
        </Pressable>
      ) : null}
      {oauthScopes ? (
        <Text style={[styles.note, muted]}>
          {translateApp(
            "These additional permissions will be requested on top of the provider defaults.",
          )}
        </Text>
      ) : null}
    </View>
  );
}

function DetailRow({ label, value }: { label: string; value: string }) {
  useAppLanguage();
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
  useAppLanguage();
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
  useAppLanguage();
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
    ><Translation message="Signed in as {{email}}" values={{ email: <Text style={{ color: colors.textPrimary }}>{email}</Text> }} />
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
