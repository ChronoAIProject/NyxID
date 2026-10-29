import { ServiceIcon } from "../../components/ServiceIcon";
import { useNativeSheetDismiss } from "../../components/workspace/useNativeSheetDismiss";
import { useRef, useState } from "react";
import {
  ActivityIndicator,
  FlatList,
  Keyboard,
  KeyboardAvoidingView,
  Modal,
  Platform,
  Pressable,
  ScrollView,
  StyleSheet,
  Switch,
  Text,
  TextInput,
  View,
} from "react-native";
import {
  Check,
  ChevronLeft,
  ChevronRight,
  Search,
  X,
} from "lucide-react-native";
import { SafeAreaProvider, SafeAreaView } from "react-native-safe-area-context";
import {
  ActionButton,
  Card,
  Label,
  Notice,
} from "../../components/workspace/WorkspaceUI";
import { fonts } from "../../theme/designTokens";
import { useTheme } from "../../theme/ThemeContext";
import { grantSignature, type AssistantAgent } from "./agentContracts";

export type AgentPermissions = Pick<
  AssistantAgent,
  "services" | "account_read" | "guest_access"
>;
type ServiceOption = {
  value: string;
  label: string;
  detail?: string;
  iconSlug?: string;
  iconUrl?: string | null;
};
type GuestLevel = "read" | "use" | "all";
const GUEST_LEVELS: {
  value: GuestLevel;
  label: string;
  description: string;
}[] = [
  {
    value: "read",
    label: "Read",
    description: "Guests can use read-only operations for this service.",
  },
  {
    value: "use",
    label: "Use",
    description:
      "Guests can look things up, create and act, but cannot change or delete existing content.",
  },
  {
    value: "all",
    label: "All granted",
    description: "Guests can use all operations already granted to this agent.",
  },
];
const guestLabel = (level: GuestLevel) =>
  GUEST_LEVELS.find((row) => row.value === level)!.label;

type Props = {
  title?: string;
  options: ServiceOption[];
  value: AgentPermissions;
  disabled: boolean;
  readOnly?: boolean;
  allowAccountRead?: boolean;
  allowGuestAccess?: boolean;
  connectionsLoading?: boolean;
  connectionsError?: boolean;
  saveError?: string;
  creating?: boolean;
  onSave: (value: AgentPermissions) => Promise<boolean>;
};

/** Editing is local to each presentation; dismissing never mutates grants. */
export function AgentPermissionsField(props: Props) {
  const { colors } = useTheme();
  const [editing, setEditing] = useState(false);
  const labels = new Map(props.options.map((row) => [row.value, row.label]));
  const count = props.value.services.length;
  const preview = props.value.services
    .slice(0, 2)
    .map((id) => labels.get(id) ?? id)
    .join(", ");
  return (
    <Card>
      <Label>{props.title ?? "Service permissions"}</Label>
      <View style={styles.summary}>
        <Text style={[styles.count, { color: colors.textPrimary }]}>
          {count} {count === 1 ? "service" : "services"} selected
        </Text>
        <Label muted numberOfLines={2}>
          {count
            ? `${preview}${count > 2 ? ` +${count - 2} more` : ""}`
            : "No service access. Choose only what this agent needs."}
        </Label>
        {(props.allowAccountRead ?? true) && (
          <Label muted>
            Account information · {props.value.account_read ? "Allowed" : "Off"}
          </Label>
        )}
      </View>
      <ActionButton
        label={props.readOnly ? "View permissions" : "Edit permissions"}
        variant="secondary"
        disabled={props.disabled && !props.readOnly}
        onPress={() => {
          if (props.disabled && !props.readOnly) return;
          Keyboard.dismiss();
          setEditing(true);
        }}
      />
      {editing && (
        <AgentPermissionsSheet {...props} onClose={() => setEditing(false)} />
      )}
    </Card>
  );
}

export function AgentPermissionsSheet({
  options,
  value,
  disabled,
  readOnly = false,
  allowAccountRead = true,
  allowGuestAccess = true,
  connectionsLoading = false,
  connectionsError = false,
  saveError,
  creating = false,
  onSave,
  onClose,
}: Props & { onClose: () => void }) {
  const { colors } = useTheme();
  const sheet = useNativeSheetDismiss(onClose);
  const [draft, setDraft] = useState<AgentPermissions>(() => ({
    services: [...value.services],
    account_read: value.account_read,
    guest_access: { ...value.guest_access },
  }));
  const [query, setQuery] = useState("");
  const [selectedOnly, setSelectedOnly] = useState(false);
  const [guestService, setGuestService] = useState<ServiceOption>();
  const [saving, setSaving] = useState(false);
  const [failedSave, setFailedSave] = useState(false);
  const [unexpectedError, setUnexpectedError] = useState<string>();
  const savingRef = useRef(false);
  const blocked = disabled || readOnly || saving;
  const known = new Set(options.map((row) => row.value));
  // Missing connections remain selectable throughout this edit, even after
  // removing them from the draft. A query refresh must never revoke access.
  const rows: ServiceOption[] = [
    ...options,
    ...[...new Set([...value.services, ...draft.services])]
      .filter((id) => !known.has(id))
      .map((id) => ({ value: id, label: id })),
  ];
  const search = query.trim().toLocaleLowerCase();
  const filtered = rows.filter(
    (row) =>
      (!selectedOnly || draft.services.includes(row.value)) &&
      `${row.label} ${row.detail ?? ""} ${row.value}`
        .toLocaleLowerCase()
        .includes(search),
  );
  const changed = grantSignature(draft) !== grantSignature(value);
  const close = () => {
    if (!savingRef.current) sheet.close();
  };
  const save = async () => {
    if (disabled || readOnly || savingRef.current || !changed) return;
    savingRef.current = true;
    setSaving(true);
    setFailedSave(false);
    setUnexpectedError(undefined);
    Keyboard.dismiss();
    try {
      const saved = await onSave({
        services: [...draft.services],
        account_read: allowAccountRead
          ? draft.account_read
          : value.account_read,
        guest_access: allowGuestAccess
          ? { ...draft.guest_access }
          : { ...value.guest_access },
      });
      if (saved) sheet.close();
      else setFailedSave(true);
    } catch (cause) {
      setUnexpectedError(
        cause instanceof Error
          ? cause.message
          : "Could not save permissions. Try again.",
      );
      setFailedSave(true);
    } finally {
      savingRef.current = false;
      setSaving(false);
    }
  };
  const header = (
    <View style={styles.listHeader}>
      <Label muted>
        {readOnly
          ? "Services granted to this specialist. These permissions are read only."
          : creating
            ? "Choose the services this specialist can use. Permissions take effect when you create the agent."
            : "Choose the services this specialist can use. Changes take effect only when you save."}
      </Label>
      {allowAccountRead && (
        <View style={[styles.account, { borderColor: colors.border }]}>
          <View style={styles.flex}>
            <Label>Read account information</Label>
            <Label muted>
              Read account metadata through existing account tools.
            </Label>
          </View>
          <Switch
            accessibilityLabel="Read account information"
            value={draft.account_read}
            disabled={blocked}
            onValueChange={(account_read) => {
              if (!disabled && !readOnly && !savingRef.current)
                setDraft((old) => ({ ...old, account_read }));
            }}
            trackColor={{ true: colors.primary }}
          />
        </View>
      )}
      {allowGuestAccess && !!draft.services.length && (
        <Label muted>
          Guest access controls how guests use a selected service in shared
          conversations.
        </Label>
      )}
      {connectionsLoading && (
        <ActivityIndicator
          accessibilityLabel="Loading services"
          color={colors.primary}
        />
      )}
      {connectionsError && (
        <Notice
          message={
            creating
              ? "Could not load available connections. You can create the agent without service access and add it later."
              : "Could not load available connections. Existing grants are preserved. Cancel and reload the agent to try again."
          }
          tone="error"
        />
      )}
    </View>
  );

  return (
    <Modal
      {...sheet.modalProps}
      presentationStyle="pageSheet"
      onRequestClose={() => {
        if (guestService && !savingRef.current) setGuestService(undefined);
        else close();
      }}
    >
      <SafeAreaProvider>
        <SafeAreaView style={[styles.fill, { backgroundColor: colors.bg }]}>
          <KeyboardAvoidingView
            style={styles.fill}
            behavior={Platform.OS === "ios" ? "padding" : "height"}
          >
            <View style={[styles.header, { borderColor: colors.border }]}>
              {guestService ? (
                <Pressable
                  accessibilityRole="button"
                  accessibilityLabel="Back to services"
                  style={styles.iconButton}
                  disabled={saving}
                  onPress={() => {
                    if (!savingRef.current) setGuestService(undefined);
                  }}
                >
                  <ChevronLeft size={22} color={colors.textPrimary} />
                </Pressable>
              ) : (
                <View style={styles.iconButton} />
              )}
              <Text
                accessibilityRole="header"
                style={[styles.heading, { color: colors.textPrimary }]}
              >
                {guestService ? "Guest access" : "Service permissions"}
              </Text>
              <Pressable
                accessibilityRole="button"
                accessibilityLabel="Cancel permission changes"
                style={styles.iconButton}
                disabled={saving}
                onPress={close}
              >
                <X size={22} color={colors.textSecondary} />
              </Pressable>
            </View>
            {guestService ? (
              <ScrollView contentContainerStyle={styles.guestContent}>
                <View
                  style={{ flexDirection: "row", alignItems: "center", gap: 8 }}
                >
                  <ServiceIcon
                    slug={guestService.iconSlug ?? guestService.value}
                    iconUrl={guestService.iconUrl}
                    size="sm"
                  />
                  <Label style={styles.serviceName}>{guestService.label}</Label>
                </View>
                <Label mono muted>
                  {guestService.value}
                </Label>
                <Label muted>
                  Choose what guests can do through this service. This does not
                  add to the agent's own permissions. Actions requiring your
                  approval still need it.
                </Label>
                {GUEST_LEVELS.map((level) => {
                  const selected =
                    (draft.guest_access[guestService.value] ?? "use") ===
                    level.value;
                  return (
                    <Pressable
                      key={level.value}
                      accessibilityRole="radio"
                      accessibilityLabel={level.label}
                      accessibilityHint={level.description}
                      accessibilityState={{
                        checked: selected,
                        disabled: blocked,
                      }}
                      disabled={blocked}
                      style={[
                        styles.guestChoice,
                        {
                          borderColor: selected
                            ? colors.primary
                            : colors.border,
                          backgroundColor: colors.card,
                        },
                      ]}
                      onPress={() => {
                        if (!disabled && !readOnly && !savingRef.current)
                          setDraft((old) => ({
                            ...old,
                            guest_access: {
                              ...old.guest_access,
                              [guestService.value]: level.value,
                            },
                          }));
                      }}
                    >
                      <View style={styles.flex}>
                        <Label>{level.label}</Label>
                        <Label muted>{level.description}</Label>
                      </View>
                      {selected && <Check size={20} color={colors.primary} />}
                    </Pressable>
                  );
                })}
              </ScrollView>
            ) : (
              <>
                <View style={styles.toolbar}>
                  <View
                    style={[
                      styles.searchBox,
                      {
                        borderColor: colors.border,
                        backgroundColor: colors.card,
                      },
                    ]}
                  >
                    <Search size={18} color={colors.textMuted} />
                    <TextInput
                      accessibilityLabel="Search service permissions"
                      placeholder="Search services"
                      placeholderTextColor={colors.textMuted}
                      value={query}
                      onChangeText={setQuery}
                      autoCapitalize="none"
                      autoCorrect={false}
                      editable={!saving}
                      style={[
                        styles.searchInput,
                        { color: colors.textPrimary },
                      ]}
                    />
                    {!!query && (
                      <Pressable
                        accessibilityRole="button"
                        accessibilityLabel="Clear service search"
                        style={styles.iconButton}
                        disabled={saving}
                        onPress={() => setQuery("")}
                      >
                        <X size={18} color={colors.textMuted} />
                      </Pressable>
                    )}
                  </View>
                  <View accessibilityRole="tablist" style={styles.tabs}>
                    {[false, true].map((only) => (
                      <Pressable
                        key={String(only)}
                        accessibilityRole="tab"
                        accessibilityLabel={
                          only ? "Selected services" : "All services"
                        }
                        accessibilityState={{ selected: selectedOnly === only }}
                        onPress={() => setSelectedOnly(only)}
                        style={[
                          styles.tab,
                          {
                            backgroundColor:
                              selectedOnly === only
                                ? colors.primaryTone.bg
                                : colors.card,
                            borderColor:
                              selectedOnly === only
                                ? colors.primaryTone.border
                                : colors.border,
                          },
                        ]}
                      >
                        <Label>
                          {only
                            ? `Selected (${draft.services.length})`
                            : `All (${rows.length})`}
                        </Label>
                      </Pressable>
                    ))}
                  </View>
                </View>
                <FlatList
                  style={styles.fill}
                  data={filtered}
                  keyExtractor={(item) => item.value}
                  keyboardShouldPersistTaps="handled"
                  keyboardDismissMode="on-drag"
                  contentContainerStyle={styles.list}
                  ListHeaderComponent={header}
                  ListEmptyComponent={
                    <Label muted>
                      {search
                        ? "No matching services. Try another name or service ID."
                        : selectedOnly
                          ? "No services selected. Choose services from All."
                          : connectionsLoading
                            ? "Loading your connected services…"
                            : "No connected services yet. You can grant access later."}
                    </Label>
                  }
                  renderItem={({ item }) => {
                    const selected = draft.services.includes(item.value);
                    return (
                      <View
                        style={[
                          styles.service,
                          {
                            borderColor: selected
                              ? colors.primaryTone.border
                              : colors.border,
                            backgroundColor: colors.card,
                          },
                        ]}
                      >
                        <Pressable
                          accessibilityRole="checkbox"
                          accessibilityLabel={item.label}
                          accessibilityHint={item.detail ?? item.value}
                          accessibilityState={{
                            checked: selected,
                            disabled: blocked,
                          }}
                          disabled={blocked}
                          style={styles.serviceToggle}
                          onPress={() => {
                            if (!disabled && !readOnly && !savingRef.current)
                              setDraft((old) => ({
                                ...old,
                                services: old.services.includes(item.value)
                                  ? old.services.filter(
                                      (id) => id !== item.value,
                                    )
                                  : [...old.services, item.value],
                              }));
                          }}
                        >
                          <ServiceIcon
                            slug={item.iconSlug ?? item.value}
                            iconUrl={item.iconUrl}
                            size="sm"
                          />
                          <View style={styles.flex}>
                            <Label>{item.label}</Label>
                            <Label mono muted style={styles.serviceId}>
                              {item.detail ?? item.value}
                            </Label>
                            {!known.has(item.value) && (
                              <Label muted>
                                Existing grant · connection unavailable
                              </Label>
                            )}
                          </View>
                          <View
                            style={[
                              styles.checkbox,
                              {
                                borderColor: selected
                                  ? colors.primary
                                  : colors.textMuted,
                                backgroundColor: selected
                                  ? colors.primary
                                  : "transparent",
                              },
                            ]}
                          >
                            {selected && (
                              <Check size={15} color={colors.onPrimary} />
                            )}
                          </View>
                        </Pressable>
                        {selected && allowGuestAccess && (
                          <Pressable
                            accessibilityRole="button"
                            accessibilityLabel={`Guest access for ${item.label}: ${guestLabel(draft.guest_access[item.value] ?? "use")}`}
                            disabled={saving || (disabled && !readOnly)}
                            style={[
                              styles.guestRow,
                              { borderColor: colors.borderSoft },
                            ]}
                            onPress={() => {
                              if (
                                (!disabled || readOnly) &&
                                !savingRef.current
                              ) {
                                Keyboard.dismiss();
                                setGuestService(item);
                              }
                            }}
                          >
                            <Label muted>Guest access</Label>
                            <View style={styles.guestValue}>
                              <Label>
                                {guestLabel(
                                  draft.guest_access[item.value] ?? "use",
                                )}
                              </Label>
                              <ChevronRight
                                size={17}
                                color={colors.textMuted}
                              />
                            </View>
                          </Pressable>
                        )}
                      </View>
                    );
                  }}
                />
              </>
            )}
            <View
              style={[
                styles.footer,
                { backgroundColor: colors.bg, borderColor: colors.border },
              ]}
            >
              {failedSave && (
                <Notice
                  message={
                    unexpectedError ??
                    saveError ??
                    "Could not save permissions. Try again."
                  }
                  tone="error"
                />
              )}
              <Label muted>
                {draft.services.length}{" "}
                {draft.services.length === 1 ? "service" : "services"} selected
                {allowAccountRead && draft.account_read
                  ? " · Account read allowed"
                  : ""}
              </Label>
              {readOnly ? (
                <Label muted>These permissions are read only.</Label>
              ) : (
                <ActionButton
                  label={
                    creating ? "Use these permissions" : "Save permissions"
                  }
                  disabled={disabled || !changed}
                  loading={saving}
                  onPress={() => void save()}
                />
              )}
            </View>
          </KeyboardAvoidingView>
        </SafeAreaView>
      </SafeAreaProvider>
    </Modal>
  );
}

const styles = StyleSheet.create({
  fill: { flex: 1 },
  flex: { flex: 1, minWidth: 0 },
  summary: { gap: 4 },
  count: { fontFamily: fonts.bodySemi, fontSize: 15, lineHeight: 22 },
  header: {
    flexDirection: "row",
    alignItems: "center",
    padding: 8,
    borderBottomWidth: 1,
  },
  heading: {
    flex: 1,
    textAlign: "center",
    fontFamily: fonts.displaySemi,
    fontSize: 18,
    lineHeight: 24,
  },
  iconButton: {
    minWidth: 44,
    minHeight: 44,
    alignItems: "center",
    justifyContent: "center",
  },
  toolbar: { paddingHorizontal: 20, paddingTop: 16, gap: 12 },
  searchBox: {
    flexDirection: "row",
    alignItems: "center",
    gap: 8,
    paddingLeft: 12,
    borderWidth: 1,
    borderRadius: 8,
  },
  searchInput: {
    flex: 1,
    minWidth: 0,
    minHeight: 44,
    paddingVertical: 8,
    paddingRight: 8,
    fontFamily: fonts.bodyRegular,
    fontSize: 14,
  },
  tabs: { flexDirection: "row", gap: 8 },
  tab: {
    minHeight: 44,
    paddingHorizontal: 14,
    paddingVertical: 10,
    borderWidth: 1,
    borderRadius: 8,
    justifyContent: "center",
  },
  list: { padding: 20, gap: 10, flexGrow: 1 },
  listHeader: { gap: 14, paddingBottom: 8 },
  account: {
    flexDirection: "row",
    alignItems: "center",
    gap: 12,
    borderWidth: 1,
    borderRadius: 12,
    padding: 12,
  },
  service: { borderWidth: 1, borderRadius: 12, overflow: "hidden" },
  serviceToggle: {
    flexDirection: "row",
    alignItems: "center",
    gap: 12,
    padding: 14,
    minHeight: 64,
  },
  serviceId: { fontSize: 11, lineHeight: 17 },
  checkbox: {
    width: 22,
    height: 22,
    borderWidth: 1.5,
    borderRadius: 5,
    alignItems: "center",
    justifyContent: "center",
  },
  guestRow: {
    flexDirection: "row",
    justifyContent: "space-between",
    alignItems: "center",
    gap: 12,
    paddingHorizontal: 14,
    minHeight: 44,
    borderTopWidth: 1,
  },
  guestValue: { flexDirection: "row", alignItems: "center", gap: 6 },
  guestContent: { padding: 20, gap: 14 },
  serviceName: { fontFamily: fonts.bodySemi, fontSize: 16, lineHeight: 24 },
  guestChoice: {
    flexDirection: "row",
    alignItems: "center",
    gap: 12,
    padding: 16,
    borderWidth: 1,
    borderRadius: 12,
  },
  footer: {
    paddingHorizontal: 20,
    paddingTop: 12,
    paddingBottom: 12,
    gap: 10,
    borderTopWidth: 1,
  },
});
