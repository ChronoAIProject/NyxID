import { useMemo, useState } from "react";
import { Pressable, Switch, Text, TextInput, View } from "react-native";
import { Check, Circle } from "lucide-react-native";
import { PrimaryButton } from "../../components/PrimaryButton";
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
  const check = (
    label: string,
    selected: boolean,
    onPress: () => void,
    radio = false,
  ) => (
    <Pressable
      key={label}
      accessibilityRole={radio ? "radio" : "checkbox"}
      accessibilityLabel={label}
      accessibilityState={{ checked: selected, disabled }}
      disabled={disabled}
      onPress={onPress}
      style={[
        styles.detailRow,
        { flexDirection: "row", alignItems: "center", gap: 12 },
      ]}
    >
      {selected ? (
        <Check size={20} color={colors.primary} />
      ) : (
        <Circle size={20} color={colors.textMuted} />
      )}
      <Text style={[styles.detailValue, { flex: 1 }]}>{label}</Text>
    </Pressable>
  );
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
      <Text style={styles.inputLabel}>Owner</Text>
      {check(
        "Personal",
        !draft.target_org_id,
        () =>
          setDraft({
            ...draft,
            target_org_id: undefined,
            allowed_service_ids: [],
            allowed_node_ids: [],
            allow_all_services: false,
            allow_all_nodes: false,
          }),
        true,
      )}
      {options.orgs.map((org) =>
        check(
          org.name,
          draft.target_org_id === org.id,
          () =>
            setDraft({
              ...draft,
              target_org_id: org.id,
              allowed_service_ids: [],
              allowed_node_ids: [],
              allow_all_services: false,
              allow_all_nodes: false,
            }),
          true,
        ),
      )}
      <Text style={styles.inputLabel}>Scopes</Text>
      {agentKeyScopes.map((scope) =>
        check(scope, draft.scopes.split(" ").includes(scope), () => {
          const selected = draft.scopes.split(" ").filter(Boolean);
          update(
            "scopes",
            (selected.includes(scope)
              ? selected.filter((item) => item !== scope)
              : [...selected, scope]
            ).join(" "),
          );
        }),
      )}
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
            {!draft[allField] &&
              resources.map((item) =>
                check(item.name, draft[idsField].includes(item.id), () =>
                  update(
                    idsField,
                    draft[idsField].includes(item.id)
                      ? draft[idsField].filter((id) => id !== item.id)
                      : [...draft[idsField], item.id],
                  ),
                ),
              )}
            {!draft[allField] && resources.length === 0 && (
              <Text style={styles.signInText}>No {kind} available.</Text>
            )}
          </View>
        );
      })}
      <Text style={styles.inputLabel}>Key expiry</Text>
      {["7", "30", "90", "365", "custom", "none"].map((choice) =>
        check(
          choice === "none"
            ? "No expiry"
            : choice === "custom"
              ? "Custom date"
              : `${choice} days`,
          expiry === choice,
          () => {
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
          },
          true,
        ),
      )}
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
      <Text style={styles.inputLabel}>Platform</Text>
      {(
        [
          ["generic", "Generic"],
          ["claude-code", "Claude Code"],
          ["cursor", "Cursor"],
          ["codex", "Codex"],
          ["openclaw", "OpenClaw"],
        ] as const
      ).map(([value, label]) =>
        check(
          label,
          (draft.platform ?? "generic") === value,
          () => update("platform", value),
          true,
        ),
      )}
      {error && (
        <Text accessibilityRole="alert" style={styles.fieldErrorText}>
          {error}
        </Text>
      )}
      <PrimaryButton
        label="Review permissions"
        disabled={disabled}
        onPress={() => {
          const data = {
            ...draft,
            expires_at:
              expiry === "custom" &&
              /^\d{4}-\d{2}-\d{2}$/.test(draft.expires_at ?? "")
                ? `${draft.expires_at}T23:59:59Z`
                : draft.expires_at,
          };
          const parsed = newAgentKeySchema.safeParse(data);
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
