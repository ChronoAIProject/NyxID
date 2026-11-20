import { useLocalizedState } from "../../i18n/useLocalizedState";
import { useAppLanguage } from "../../i18n/LanguageProvider";
import { translateApp } from "../../i18n/appTranslation";
import { ServiceIcon } from "../../components/ServiceIcon";
import { useNativeSheetDismiss } from "../../components/workspace/useNativeSheetDismiss";
import { useRef, useState } from "react";
import {
  FlatList,
  Keyboard,
  Modal,
  Pressable,
  ScrollView,
  StyleSheet,
  Switch,
  Text,
  TextInput,
  View,
  useWindowDimensions,
} from "react-native";
import {
  Check,
  ChevronLeft,
  ChevronRight,
  Search,
  X,
} from "lucide-react-native";
import { SafeAreaProvider, SafeAreaView } from "react-native-safe-area-context";
import { KeyboardViewport } from "../../components/workspace/KeyboardViewport";
import { SkeletonRows } from "../../components/Skeleton";
import {
  SettingsGroup,
  SettingsRow,
} from "../../components/settings/SettingsList";
import {
  ActionButton,
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
    get label() {
      return translateApp("Read");
    },
    get description() {
      return translateApp(
        "Guests can use read-only operations for this service.",
      );
    },
  },
  {
    value: "use",
    get label() {
      return translateApp("Use");
    },
    get description() {
      return translateApp(
        "Guests can look things up, create and act, but cannot change or delete existing content.",
      );
    },
  },
  {
    value: "all",
    get label() {
      return translateApp("All granted");
    },
    get description() {
      return translateApp(
        "Guests can use all operations already granted to this agent.",
      );
    },
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
  useAppLanguage();
  const [editing, setEditing] = useState(false);
  const labels = new Map(props.options.map((row) => [row.value, row.label]));
  const count = props.value.services.length;
  const preview = props.value.services
    .slice(0, 2)
    .map((id) => labels.get(id) ?? id)
    .join(", ");
  const blocked = props.disabled && !props.readOnly;
  const open = () => {
    if (blocked) return;
    Keyboard.dismiss();
    setEditing(true);
  };
  const hint = props.readOnly ? translateApp("Shows the granted permissions")
    : translateApp("Edits permissions");
  return (
    <>
      <SettingsGroup inset={false} title={props.title ?? translateApp("Service permissions")}>
        <SettingsRow
          label={translateApp("Services")}
          value={(count === 1 ? translateApp("{{count}} service selected", { count: count }) : translateApp("{{count}} services selected", { count: count }))}
          detail={
            count
              ? (count > 2 ? translateApp("{{preview}} +{{count}} more", { preview: preview, count: count - 2}) : preview)
              : translateApp(
                  "No service access. Choose only what this agent needs.",
                )
          }
          accessibilityLabel={props.readOnly ? translateApp("View permissions")
              : translateApp("Edit permissions")
          }
          accessibilityHint={hint}
          disabled={blocked}
          onPress={open}
        />
        {(props.allowAccountRead ?? true) && (
          <SettingsRow
            label={translateApp("Account information")}
            value={props.value.account_read ? translateApp("Allowed")
                : translateApp("Off")
            }
            accessibilityHint={hint}
            disabled={blocked}
            onPress={open}
          />
        )}
      </SettingsGroup>
      {editing && (
        <AgentPermissionsSheet {...props} onClose={() => setEditing(false)} />
      )}
    </>
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
  useAppLanguage();
  const { colors } = useTheme();
  const sheet = useNativeSheetDismiss(onClose);
  const [draft, setDraft] = useState<AgentPermissions>(() => ({
    services: [...value.services],
    account_read: value.account_read,
    guest_access: { ...value.guest_access },
  }));
  const [query, setQuery] = useState("");
  const { fontScale } = useWindowDimensions();
  const [compactSearch, setCompactSearch] = useState(false);
  const [selectedOnly, setSelectedOnly] = useState(false);
  const [guestService, setGuestService] = useState<ServiceOption>();
  const [saving, setSaving] = useState(false);
  const [failedSave, setFailedSave] = useState(false);
  const [unexpectedError, setUnexpectedError] = useLocalizedState<string>();
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
        () => (cause instanceof Error
          ? cause.message
          : translateApp("Could not save permissions. Try again.")),
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
          ? translateApp(
              "Services granted to this specialist. These permissions are read only.",
            )
          : creating
            ? translateApp(
                "Choose the services this specialist can use. Permissions take effect when you create the agent.",
              )
            : translateApp(
                "Choose the services this specialist can use. Changes take effect only when you save.",
              )}
      </Label>
      {allowAccountRead && (
        <View style={[styles.account, { borderColor: colors.border }]}>
          <View style={styles.flex}>
            <Label>{translateApp("Read account information")}</Label>
            <Label muted>
              {translateApp(
                "Read account metadata through existing account tools.",
              )}
            </Label>
          </View>
          <Switch
            accessibilityLabel={translateApp("Read account information")}
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
          {translateApp(
            "Guest access controls how guests use a selected service in shared conversations.",
          )}
        </Label>
      )}
      {connectionsLoading && (
        <SkeletonRows rows={3} accessibilityLabel={translateApp("Loading services")}
        />
      )}
      {connectionsError && (
        <Notice
          message={
            creating
              ? translateApp(
                  "Could not load available connections. You can create the agent without service access and add it later.",
                )
              : translateApp(
                  "Could not load available connections. Existing grants are preserved. Cancel and reload the agent to try again.",
                )
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
          <KeyboardViewport
            onViewportChange={(frame) =>
              // Preserve two usable result rows after the fixed controls.
              // Tall windows keep their ordinary filters and save footer.
              setCompactSearch(Boolean(frame && frame.height < 420 * Math.max(1, fontScale)))
            }
          >
            <View style={[styles.header, { borderColor: colors.border }]}>
              {guestService ? (
                <Pressable
                  accessibilityRole="button"
                  accessibilityLabel={translateApp("Back to services")}
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
                {guestService ? translateApp("Guest access")
                  : translateApp("Service permissions")}
              </Text>
              <Pressable
                accessibilityRole="button"
                accessibilityLabel={translateApp("Cancel permission changes")}
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
                  {translateApp(
                    "Choose what guests can do through this service. This does not add to the agent's own permissions. Actions requiring your approval still need it.",
                  )}
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
                      accessibilityLabel={translateApp(
                        "Search service permissions",
                      )}
                      placeholder={translateApp("Search services")}
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
                        accessibilityLabel={translateApp(
                          "Clear service search",
                        )}
                        style={styles.iconButton}
                        disabled={saving}
                        onPress={() => setQuery("")}
                      >
                        <X size={18} color={colors.textMuted} />
                      </Pressable>
                    )}
                  </View>
                  {!compactSearch && <View accessibilityRole="tablist" style={styles.tabs}>
                    {[false, true].map((only) => (
                      <Pressable
                        key={String(only)}
                        accessibilityRole="tab"
                        accessibilityLabel={
                          only ? translateApp("Selected services")
                            : translateApp("All services")
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
                            ? translateApp("Selected ({{count}})", {
                                count: draft.services.length,
                              })
                            : translateApp("All ({{count}})", {
                                count: rows.length,
                              })}
                        </Label>
                      </Pressable>
                    ))}
                  </View>}
                </View>
                <FlatList
                  style={styles.fill}
                  data={filtered}
                  keyExtractor={(item) => item.value}
                  keyboardShouldPersistTaps="handled"
                  keyboardDismissMode={compactSearch ? "none" : "on-drag"}
                  contentContainerStyle={styles.list}
                  ListHeaderComponent={compactSearch ? null : header}
                  ListEmptyComponent={
                    // While connections load, the header's skeleton rows
                    // stand in for the list.
                    !search && !selectedOnly && connectionsLoading ? null : (
                      <Label muted>
                        {search
                          ? translateApp(
                              "No matching services. Try another name or service ID.",
                            )
                          : selectedOnly
                            ? translateApp(
                                "No services selected. Choose services from All.",
                              )
                            : translateApp(
                                "No connected services yet. You can grant access later.",
                              )}
                      </Label>
                    )
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
                                {translateApp(
                                  "Existing grant · connection unavailable",
                                )}
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
                            accessibilityLabel={translateApp(
                              "Guest access for {{service}}: {{access}}",
                              {
                                service: item.label,
                                access: guestLabel(draft.guest_access[item.value] ?? "use"),
                              },
                            )}
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
                            <Label muted>{translateApp("Guest access")}</Label>
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
            {(!compactSearch || guestService) && <View
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
                    translateApp("Could not save permissions. Try again.")
                  }
                  tone="error"
                />
              )}
              <Label muted>
                {(draft.services.length === 1 ? (allowAccountRead && draft.account_read
                  ? translateApp("{{count}} service selected · Account read allowed", { count: draft.services.length }) : translateApp("{{count}} service selected", { count: draft.services.length })) : (allowAccountRead && draft.account_read ? translateApp("{{count}} services selected · Account read allowed", { count: draft.services.length }) : translateApp("{{count}} services selected", { count: draft.services.length })))}
              </Label>
              {readOnly ? (
                <Label muted>
                  {translateApp("These permissions are read only.")}
                </Label>
              ) : (
                <ActionButton
                  label={
                    creating ? translateApp("Use these permissions")
                      : translateApp("Save permissions")
                  }
                  disabled={disabled || !changed}
                  loading={saving}
                  onPress={() => void save()}
                />
              )}
            </View>}
          </KeyboardViewport>
        </SafeAreaView>
      </SafeAreaProvider>
    </Modal>
  );
}

const styles = StyleSheet.create({
  fill: { flex: 1, minHeight: 0 },
  flex: { flex: 1, minWidth: 0 },
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
