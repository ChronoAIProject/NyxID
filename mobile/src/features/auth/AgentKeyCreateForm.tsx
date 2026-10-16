import { useMemo, useState } from "react";
import { Switch, Text, TextInput, View } from "react-native";
import { PrimaryButton } from "../../components/PrimaryButton";
import { WorkspaceSelectionField } from "../../components/workspace/WorkspaceSelectionField";
import { useTheme } from "../../theme/ThemeContext";
import {
  agentKeyScopes,
  newAgentKeySchema,
  type AgentKeyOptions,
  type NewAgentKey,
} from "../../lib/api/agentKeyLoginSchema";
import { createDeviceLoginStyles } from "./deviceLoginStyles";
import { defaultNewAgentKey } from "./agentKeyLoginModel";

export function AgentKeyCreateForm({
  options,
  initialValues,
  disabled,
  onReview,
}: {
  options: AgentKeyOptions;
  initialValues?: NewAgentKey;
  disabled: boolean;
  onReview: (input: NewAgentKey) => void;
}) {
  const { colors } = useTheme();
  const styles = useMemo(() => createDeviceLoginStyles(colors), [colors]);
  const [draft, setDraft] = useState(initialValues ?? defaultNewAgentKey);
  const [expiry, setExpiry] = useState(
    initialValues ? (initialValues.expires_at ? "custom" : "none") : "90",
  );
  const [error, setError] = useState<string | null>(null);
  function update<K extends keyof NewAgentKey>(key: K, value: NewAgentKey[K]) {
    setDraft((current) => ({ ...current, [key]: value }));
  }
  return (
    <View style={styles.inputSection}>
      <Text style={styles.inputLabel}>Name</Text>
      <TextInput
        accessibilityLabel="Key name"
        value={draft.name}
        maxLength={64}
        onChangeText={(value) => update("name", value)}
        editable={!disabled}
        style={[styles.codeInput, { textAlign: "left" }]}
      />
      <WorkspaceSelectionField
        label="Owner"
        title="Choose key owner"
        options={[
          { value: "personal", label: "Personal" },
          ...options.orgs.map((org) => ({
            value: org.id,
            label: org.name,
            description: "Organization",
            searchText: org.id,
          })),
        ]}
        value={[draft.target_org_id ?? "personal"]}
        disabled={disabled}
        onChange={([owner]) => {
          if (!owner || owner === (draft.target_org_id ?? "personal")) return;
          setDraft({
            ...draft,
            target_org_id: owner === "personal" ? undefined : owner,
            allowed_service_ids: [],
            allowed_node_ids: [],
            allow_all_services: false,
            allow_auto_connected_services: false,
            allow_all_nodes: false,
          });
        }}
      />
      <WorkspaceSelectionField
        label="Scopes"
        title="Choose key scopes"
        options={agentKeyScopes.map((scope) => ({
          value: scope,
          label: scope,
        }))}
        value={draft.scopes.split(/\s+/).filter(Boolean)}
        multiple
        disabled={disabled}
        onChange={(scopes) => update("scopes", scopes.join(" "))}
      />
      {(["services", "nodes"] as const).map((kind) => {
        const allField =
          kind === "services" ? "allow_all_services" : "allow_all_nodes";
        const idsField =
          kind === "services" ? "allowed_service_ids" : "allowed_node_ids";
        const resources = options[kind].filter(
          (item) =>
            !draft.target_org_id || item.owner_id === draft.target_org_id,
        );
        return (
          <View key={kind}>
            <View
              style={[
                styles.detailRow,
                {
                  flexDirection: "row",
                  alignItems: "center",
                  justifyContent: "space-between",
                },
              ]}
            >
              <Text style={styles.inputLabel}>Allow all {kind}</Text>
              <Switch
                accessibilityLabel={`Allow all ${kind}`}
                disabled={disabled}
                value={draft[allField]}
                onValueChange={(value) =>
                  setDraft({ ...draft, [allField]: value, [idsField]: [] })
                }
              />
            </View>
            {!draft[allField] && (
              <WorkspaceSelectionField
                label={kind === "services" ? "Service access" : "Node access"}
                title={kind === "services" ? "Choose services" : "Choose nodes"}
                options={resources.map((item) => {
                  const sameOwner =
                    item.owner_id ===
                    (draft.target_org_id || options.personal_owner_id);
                  const implied =
                    kind === "services" &&
                    !!item.auto_connected &&
                    draft.allow_auto_connected_services &&
                    sameOwner;
                  return {
                    value: item.id,
                    label: item.name,
                    description: implied
                      ? "Included by auto-connected platform access"
                      : item.auto_connected
                        ? sameOwner
                          ? "Platform service"
                          : "Organization platform service; select individually"
                        : undefined,
                    searchText: item.id,
                    disabled: implied,
                  };
                })}
                value={[
                  ...draft[idsField],
                  ...resources
                    .filter(
                      (item) =>
                        kind === "services" &&
                        item.auto_connected &&
                        draft.allow_auto_connected_services &&
                        item.owner_id ===
                          (draft.target_org_id || options.personal_owner_id) &&
                        !draft[idsField].includes(item.id),
                    )
                    .map((item) => item.id),
                ]}
                multiple
                scopeKey={draft.target_org_id ?? "personal"}
                disabled={disabled}
                onChange={(ids) => {
                  const impliedIds = new Set(
                    resources
                      .filter(
                        (item) =>
                          kind === "services" &&
                          item.auto_connected &&
                          draft.allow_auto_connected_services &&
                          item.owner_id ===
                            (draft.target_org_id || options.personal_owner_id),
                      )
                      .map((item) => item.id),
                  );
                  update(
                    idsField,
                    ids.filter(
                      (id) =>
                        !impliedIds.has(id) || draft[idsField].includes(id),
                    ),
                  );
                }}
              />
            )}
            {kind === "services" &&
              !draft[allField] &&
              (draft.target_org_id &&
              !resources.some((item) => item.auto_connected) ? (
                <Text style={styles.signInText}>
                  This org-owned key cannot use platform services from your
                  personal account.
                </Text>
              ) : (
                <View
                  style={[
                    styles.detailRow,
                    {
                      flexDirection: "row",
                      alignItems: "center",
                      gap: 12,
                    },
                  ]}
                >
                  <Text style={[styles.detailValue, { flex: 1 }]}>
                    Allow all auto-connected platform services (includes ones
                    added later)
                  </Text>
                  <Switch
                    accessibilityLabel="Allow all auto-connected platform services (includes ones added later)"
                    disabled={disabled}
                    value={draft.allow_auto_connected_services}
                    onValueChange={(value) =>
                      update("allow_auto_connected_services", value)
                    }
                  />
                </View>
              ))}
            {!draft[allField] && resources.length === 0 && (
              <Text style={styles.signInText}>No {kind} available.</Text>
            )}
          </View>
        );
      })}
      <WorkspaceSelectionField
        label="Key expiry"
        title="Choose key expiry"
        options={["7", "30", "90", "365", "custom", "none"].map((choice) => ({
          value: choice,
          label:
            choice === "none"
              ? "No expiry"
              : choice === "custom"
                ? "Custom date"
                : `${choice} days`,
        }))}
        value={[expiry]}
        disabled={disabled}
        onChange={([choice]) => {
          if (!choice || choice === expiry) return;
          setExpiry(choice);
          update(
            "expires_at",
            choice === "none"
              ? null
              : choice === "custom"
                ? ""
                : new Date(
                    Date.now() + Number(choice) * 86400000,
                  ).toISOString(),
          );
        }}
      />
      {expiry === "custom" && (
        <TextInput
          accessibilityLabel="Custom key expiry"
          placeholder="YYYY-MM-DD"
          placeholderTextColor={colors.textMuted}
          value={draft.expires_at ?? ""}
          editable={!disabled}
          onChangeText={(value) => update("expires_at", value)}
          style={styles.codeInput}
        />
      )}
      {(["rate_limit_per_second", "rate_limit_burst"] as const).map((field) => (
        <View key={field} style={styles.inputSection}>
          <Text style={styles.inputLabel}>
            {field === "rate_limit_per_second"
              ? "Requests per second"
              : "Burst"}
          </Text>
          <TextInput
            accessibilityLabel={
              field === "rate_limit_per_second"
                ? "Requests per second"
                : "Burst"
            }
            keyboardType="number-pad"
            value={draft[field]?.toString() ?? ""}
            placeholder="Default"
            placeholderTextColor={colors.textMuted}
            editable={!disabled}
            onChangeText={(value) =>
              update(field, value ? Number(value) : undefined)
            }
            style={styles.codeInput}
          />
        </View>
      ))}
      <WorkspaceSelectionField
        label="Platform"
        title="Choose client platform"
        options={[
          { value: "generic", label: "Generic" },
          { value: "claude-code", label: "Claude Code" },
          { value: "cursor", label: "Cursor" },
          { value: "codex", label: "Codex" },
          { value: "openclaw", label: "OpenClaw" },
        ]}
        value={[draft.platform ?? "generic"]}
        disabled={disabled}
        onChange={([platform]) => {
          if (platform) update("platform", platform);
        }}
      />
      {error && (
        <Text accessibilityRole="alert" style={styles.fieldErrorText}>
          {error}
        </Text>
      )}
      <PrimaryButton
        label="Review permissions"
        disabled={disabled}
        onPress={() => {
          const parsed = newAgentKeySchema.safeParse(draft);
          if (expiry === "custom" && !draft.expires_at) {
            setError("Choose a custom expiry date.");
            return;
          }
          if (!parsed.success) {
            setError(
              parsed.error.issues[0]?.message ?? "Check the key details.",
            );
            return;
          }
          setError(null);
          onReview(parsed.data);
        }}
      />
    </View>
  );
}
