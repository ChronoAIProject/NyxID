import {
  ActivityIndicator,
  Keyboard,
  Pressable,
  StyleSheet,
  Text,
  TextInput,
  View,
} from "react-native";
import { BookOpen, Check, ChevronRight, Search, X } from "lucide-react-native";
import { SegmentControl } from "../../components/SegmentControl";
import {
  ActionButton,
  Label,
  Notice,
} from "../../components/workspace/WorkspaceUI";
import { fonts } from "../../theme/designTokens";
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
      <View
        style={[
          styles.search,
          { backgroundColor: colors.card, borderColor: colors.border },
        ]}
      >
        <Search size={18} color={colors.textMuted} accessible={false} />
        <TextInput
          accessibilityLabel={
            tab === "added" ? "Search added skills" : "Search skill catalog"
          }
          placeholder={
            tab === "added" ? "Search added skills…" : "Search skill catalog…"
          }
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
            <X size={17} color={colors.textSecondary} />
          </Pressable>
        )}
      </View>
      <View style={styles.heading}>
        <Text
          accessibilityRole="header"
          style={[styles.sectionTitle, { color: colors.textPrimary }]}
        >
          {tab === "added" ? "Added to this agent" : "Skill catalog"}
        </Text>
        {tab === "added" && added.known && (
          <Label muted style={styles.caption}>
            {added.items.length} of 16
          </Label>
        )}
      </View>
      <Text style={[styles.help, { color: colors.textSecondary }]}>
        {tab === "added"
          ? "Open a skill to view its instructions or manage its version."
          : `Find a skill for ${agentName}. Review its version before adding.`}
      </Text>
      {list.error && (
        <>
          <Notice tone="error" message={list.error} />
          <ActionButton
            label={tab === "added" ? "Reload skills" : "Retry skill catalog"}
            variant="secondary"
            onPress={list.onRetry}
          />
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
        <>
          {filtered.map((item) => (
            <SkillListRow
              key={item.id}
              item={item}
              onPress={() => onOpenSkill(item.id, item.name, item.description)}
            />
          ))}
          {added.known &&
            !added.loading &&
            !added.error &&
            !filtered.length && (
              <View
                style={[styles.empty, { backgroundColor: colors.cardSoft }]}
              >
                <BookOpen size={26} color={colors.primary} />
                <Text
                  style={[styles.emptyTitle, { color: colors.textPrimary }]}
                >
                  {needle ? "No matching added skills" : "No skills added yet"}
                </Text>
                <Text
                  style={[
                    styles.help,
                    styles.center,
                    { color: colors.textSecondary },
                  ]}
                >
                  {needle
                    ? "Try a different name or description, or search the catalog."
                    : "Add a skill to teach this agent a workflow. Skills do not grant service access."}
                </Text>
                <ActionButton
                  label="Discover skills"
                  variant="secondary"
                  onPress={() => {
                    Keyboard.dismiss();
                    onTabChange("discover");
                  }}
                />
              </View>
            )}
        </>
      ) : (
        <>
          {catalog.items.map((item) => (
            <SkillListRow
              key={item.id}
              item={item}
              attached={added.items.some((pin) => pin.id === item.id)}
              onPress={() => onOpenSkill(item.id, item.name, item.description)}
            />
          ))}
          {catalog.known &&
            !catalog.loading &&
            !catalog.error &&
            !catalog.items.length && (
              <View
                style={[styles.empty, { backgroundColor: colors.cardSoft }]}
              >
                <Search size={26} color={colors.textMuted} />
                <Text
                  style={[styles.emptyTitle, { color: colors.textPrimary }]}
                >
                  No skills found
                </Text>
                <Text
                  style={[
                    styles.help,
                    styles.center,
                    { color: colors.textSecondary },
                  ]}
                >
                  Try another keyword. Only skills available through your Ornn
                  access appear here.
                </Text>
              </View>
            )}
          {catalog.known && totalPages > 1 && (
            <View style={styles.pagination}>
              <View style={styles.flex}>
                <ActionButton
                  label="Previous"
                  variant="secondary"
                  disabled={catalog.loading || page <= 1}
                  onPress={() => onPageChange(page - 1)}
                />
              </View>
              <Label muted style={styles.caption}>
                {page} / {totalPages}
              </Label>
              <View style={styles.flex}>
                <ActionButton
                  label="Next"
                  variant="secondary"
                  disabled={catalog.loading || page >= totalPages}
                  onPress={() => onPageChange(page + 1)}
                />
              </View>
            </View>
          )}
        </>
      )}
    </View>
  );
}

function SkillListRow({
  item,
  attached,
  onPress,
}: {
  item: CatalogSkill | AddedSkillItem;
  attached?: boolean;
  onPress: () => void;
}) {
  const { colors } = useTheme();
  const pin = "version" in item ? item : undefined;
  const state = pin
    ? pin.disabled
      ? "Disabled"
      : "Enabled"
    : attached
      ? "Added"
      : "View versions";
  return (
    <Pressable
      accessibilityRole="button"
      accessibilityLabel={`${item.name}${pin ? `, version ${pin.version}` : ""}, ${state}`}
      accessibilityHint="View skill details"
      onPress={onPress}
      style={({ pressed }) => [
        styles.skill,
        {
          backgroundColor: pressed ? colors.cardSoft : colors.card,
          borderColor: colors.border,
        },
      ]}
    >
      <View style={styles.skillTop}>
        <View style={[styles.icon, { backgroundColor: colors.primaryTone.bg }]}>
          <BookOpen size={18} color={colors.primaryOnTint} />
        </View>
        <Text
          numberOfLines={2}
          style={[styles.skillName, { color: colors.textPrimary }]}
        >
          {skillDisplayName(item.name)}
        </Text>
        <ChevronRight size={17} color={colors.textMuted} />
      </View>
      <Text
        numberOfLines={2}
        style={[styles.description, { color: colors.textSecondary }]}
      >
        {item.description?.trim() ||
          "Open this skill to review its details and available versions."}
      </Text>
      <View style={styles.metadata}>
        {pin && (
          <Text
            style={[
              styles.version,
              { backgroundColor: colors.cardSoft, color: colors.textSecondary },
            ]}
          >
            Version {pin.version}
          </Text>
        )}
        {attached && <Check size={13} color={colors.success} />}
        <Text
          style={[
            styles.caption,
            {
              color: pin?.disabled
                ? colors.textMuted
                : attached
                  ? colors.success
                  : colors.textSecondary,
            },
          ]}
        >
          {state}
        </Text>
      </View>
    </Pressable>
  );
}

const styles = StyleSheet.create({
  stack: { gap: 12 },
  tabs: { marginBottom: 0 },
  search: {
    flexDirection: "row",
    alignItems: "center",
    gap: 9,
    borderWidth: 1,
    borderRadius: 10,
    paddingLeft: 12,
    paddingRight: 4,
  },
  searchInput: {
    flex: 1,
    minWidth: 0,
    minHeight: 46,
    fontFamily: fonts.body,
    fontSize: 13,
    lineHeight: 20,
    paddingVertical: 10,
  },
  clear: {
    minWidth: 44,
    minHeight: 44,
    justifyContent: "center",
    alignItems: "center",
  },
  heading: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
    gap: 12,
    marginTop: 6,
  },
  sectionTitle: {
    fontFamily: fonts.bodySemi,
    fontSize: 14,
    lineHeight: 20,
    flexShrink: 1,
  },
  help: { fontFamily: fonts.bodyRegular, fontSize: 12, lineHeight: 18 },
  caption: { fontFamily: fonts.body, fontSize: 11, lineHeight: 17 },
  skill: { borderWidth: 1, borderRadius: 12, padding: 14, gap: 10 },
  skillTop: { flexDirection: "row", alignItems: "center", gap: 10 },
  skillName: {
    flex: 1,
    minWidth: 0,
    fontFamily: fonts.bodySemi,
    fontSize: 14,
    lineHeight: 20,
  },
  icon: {
    width: 32,
    height: 32,
    borderRadius: 8,
    alignItems: "center",
    justifyContent: "center",
    flexShrink: 0,
  },
  description: { fontFamily: fonts.bodyRegular, fontSize: 12, lineHeight: 18 },
  metadata: {
    flexDirection: "row",
    alignItems: "center",
    gap: 6,
    flexWrap: "wrap",
  },
  version: {
    fontFamily: fonts.mono,
    fontSize: 10,
    lineHeight: 16,
    paddingHorizontal: 6,
    paddingVertical: 2,
    borderRadius: 5,
  },
  empty: { padding: 20, gap: 10, alignItems: "center", borderRadius: 12 },
  emptyTitle: { fontFamily: fonts.bodySemi, fontSize: 14, lineHeight: 20 },
  center: { textAlign: "center" },
  flex: { flex: 1 },
  pagination: { flexDirection: "row", alignItems: "center", gap: 10 },
});
