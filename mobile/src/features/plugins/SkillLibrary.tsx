import {
  ActivityIndicator,
  Keyboard,
  Pressable,
  StyleSheet,
  TextInput,
  View,
} from "react-native";
import { BookOpen, Search, X } from "lucide-react-native";
import { SegmentControl } from "../../components/SegmentControl";
import {
  SettingsGroup,
  SettingsRow,
} from "../../components/settings/SettingsList";
import { Notice } from "../../components/workspace/WorkspaceUI";
import { radius, spacing, typeScale } from "../../theme/designTokens";
import { useTheme } from "../../theme/ThemeContext";
import type { CatalogSkill } from "./pluginSkillsTypes";

export type SkillLibraryTab = "added" | "discover";
export type AddedSkillItem = CatalogSkill & {
  version: string;
  disabled: boolean;
};
type ListState<T> = {
  items: readonly T[];
  loading: boolean;
  known: boolean;
  error?: string;
  onRetry: () => void;
};

/** Make slug-style names readable without changing their exact identity. */
export function skillDisplayName(name: string) {
  if (!/^[a-z0-9]+(?:[-_][a-z0-9]+)+$/.test(name)) return name;
  const words = name.replace(/[-_]/g, " ");
  return words.charAt(0).toUpperCase() + words.slice(1);
}

function searchable(value: string) {
  return value
    .toLocaleLowerCase()
    .replace(/[-_]/g, " ")
    .replace(/\s+/g, " ")
    .trim();
}

export function SkillLibrary({
  agentName,
  tab,
  onTabChange,
  query,
  onQueryChange,
  added,
  catalog,
  page,
  totalPages,
  onPageChange,
  onOpenSkill,
}: {
  agentName: string;
  tab: SkillLibraryTab;
  onTabChange: (tab: SkillLibraryTab) => void;
  query: string;
  onQueryChange: (query: string) => void;
  added: ListState<AddedSkillItem>;
  catalog: ListState<CatalogSkill>;
  page: number;
  totalPages: number;
  onPageChange: (page: number) => void;
  onOpenSkill: (id: string, name: string, description?: string | null) => void;
}) {
  const { colors } = useTheme();
  const needle = searchable(query);
  const filtered = added.items.filter((skill) =>
    searchable(`${skill.name} ${skill.description}`).includes(needle),
  );
  const list = tab === "added" ? added : catalog;
  const addedTitle = `Added to this agent${added.known ? ` · ${added.items.length} of 16` : ""}`;
  return (
    <View style={styles.stack}>
      <SegmentControl
        segments={[
          {
            id: "added",
            label: `Added${added.known ? ` · ${added.items.length}` : ""}`,
            accessibilityLabel: "Added skills",
          },
          {
            id: "discover",
            label: "Discover",
            accessibilityLabel: "Discover skills",
          },
        ]}
        activeIndex={tab === "added" ? 0 : 1}
        onPress={(index) => {
          Keyboard.dismiss();
          onTabChange(index === 0 ? "added" : "discover");
        }}
        style={styles.tabs}
      />
      <View style={[styles.search, { backgroundColor: colors.card }]}>
        <Search size={17} color={colors.textMuted} accessible={false} />
        <TextInput
          accessibilityLabel={
            tab === "added" ? "Search added skills" : "Search skill catalog"
          }
          placeholder={tab === "added" ? "Search added skills" : "Search catalog"}
          placeholderTextColor={colors.textMuted}
          selectionColor={colors.primary}
          value={query}
          onChangeText={onQueryChange}
          maxLength={200}
          autoCapitalize="none"
          autoCorrect={false}
          returnKeyType="search"
          onSubmitEditing={Keyboard.dismiss}
          style={[styles.searchInput, { color: colors.textPrimary }]}
        />
        {query.length > 0 && (
          <Pressable
            accessibilityRole="button"
            accessibilityLabel="Clear skill search"
            onPress={() => onQueryChange("")}
            style={styles.clear}
          >
            <X size={16} color={colors.textMuted} />
          </Pressable>
        )}
      </View>
      {list.error && (
        <>
          <Notice tone="error" message={list.error} />
          <SettingsGroup inset={false}>
            <SettingsRow
              label={tab === "added" ? "Reload skills" : "Retry skill catalog"}
              tone="action"
              onPress={list.onRetry}
            />
          </SettingsGroup>
        </>
      )}
      {list.loading && (
        <ActivityIndicator
          accessibilityLabel={
            tab === "added" ? "Loading added skills" : "Searching skill catalog"
          }
          color={colors.primary}
        />
      )}
      {tab === "added" ? (
        filtered.length ? (
          <SettingsGroup
            title={addedTitle}
            footer="Open a skill to view its instructions or manage its version."
          >
            {filtered.map((item) => (
              <SkillListRow
                key={item.id}
                item={item}
                onPress={() => onOpenSkill(item.id, item.name, item.description)}
              />
            ))}
          </SettingsGroup>
        ) : (
          added.known &&
          !added.loading &&
          !added.error && (
            <SettingsGroup
              title={addedTitle}
              footer={
                needle
                  ? "Try a different name or description, or search the catalog."
                  : "Add a skill to teach this agent a workflow. Skills do not grant service access."
              }
            >
              <SettingsRow
                icon={BookOpen}
                label={needle ? "No matching added skills" : "No skills added yet"}
              />
              <SettingsRow
                label="Discover skills"
                tone="action"
                onPress={() => {
                  Keyboard.dismiss();
                  onTabChange("discover");
                }}
              />
            </SettingsGroup>
          )
        )
      ) : (
        <>
          {catalog.items.length ? (
            <SettingsGroup
              title="Skill catalog"
              footer={`Find a skill for ${agentName}. Review its version before adding.`}
            >
              {catalog.items.map((item) => (
                <SkillListRow
                  key={item.id}
                  item={item}
                  attached={added.items.some((pin) => pin.id === item.id)}
                  onPress={() =>
                    onOpenSkill(item.id, item.name, item.description)
                  }
                />
              ))}
            </SettingsGroup>
          ) : (
            catalog.known &&
            !catalog.loading &&
            !catalog.error && (
              <SettingsGroup
                title="Skill catalog"
                footer="Try another keyword. Only skills available through your Ornn access appear here."
              >
                <SettingsRow icon={Search} label="No skills found" />
              </SettingsGroup>
            )
          )}
          {catalog.known && totalPages > 1 && (
            <SettingsGroup title={`Page ${page} / ${totalPages}`} inset={false}>
              <SettingsRow
                label="Previous"
                tone="action"
                accessibilityLabel="Previous"
                accessibilityHint="Shows the previous page of results"
                disabled={catalog.loading || page <= 1}
                onPress={() => onPageChange(page - 1)}
              />
              <SettingsRow
                label="Next"
                tone="action"
                accessibilityLabel="Next"
                accessibilityHint="Shows the next page of results"
                disabled={catalog.loading || page >= totalPages}
                onPress={() => onPageChange(page + 1)}
              />
            </SettingsGroup>
          )}
        </>
      )}
    </View>
  );
}

/** A skill row: name, two-line description, and its pinned version or state. */
function SkillListRow({
  item,
  attached,
  onPress,
}: {
  item: CatalogSkill | AddedSkillItem;
  attached?: boolean;
  onPress: () => void;
}) {
  const pin = "version" in item ? item : undefined;
  const state = pin
    ? pin.disabled
      ? "Disabled"
      : "Enabled"
    : attached
      ? "Added"
      : "View versions";
  return (
    <SettingsRow
      icon={BookOpen}
      label={skillDisplayName(item.name)}
      detail={
        item.description?.trim() ||
        "Open this skill to review its details and available versions."
      }
      detailLines={2}
      value={
        pin
          ? pin.disabled
            ? `${pin.version} · Disabled`
            : pin.version
          : attached
            ? "Added"
            : undefined
      }
      accessibilityLabel={`${item.name}${pin ? `, version ${pin.version}` : ""}, ${state}`}
      accessibilityHint="View skill details"
      onPress={onPress}
    />
  );
}

const styles = StyleSheet.create({
  stack: { gap: spacing.lg },
  tabs: { marginBottom: 0 },
  search: {
    flexDirection: "row",
    alignItems: "center",
    gap: spacing.sm - 2,
    borderRadius: radius.md + 2,
    paddingLeft: spacing.md,
    paddingRight: spacing.xs,
  },
  searchInput: {
    flex: 1,
    minWidth: 0,
    minHeight: 40,
    paddingVertical: spacing.sm,
    ...typeScale.label,
  },
  clear: {
    minWidth: 44,
    minHeight: 44,
    justifyContent: "center",
    alignItems: "center",
  },
});
