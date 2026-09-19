import { useFormContext } from "react-hook-form";
import type { BillingTargetKind } from "@/schemas/billing-credits";
import {
  FormControl,
  FormDescription,
  FormField,
  FormItem,
  FormLabel,
  FormMessage,
} from "@/components/ui/form";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { GroupPicker, OrgPicker, UserPicker } from "./credit-pickers";

type RecipientTargetValues = {
  target_kind: BillingTargetKind;
  target_user_ids: string[];
  target_org_ids?: string[];
  target_group_ids?: string[];
};

/** All credit dialogs provide these recipient fields through their Form. */
export function RecipientTargetFields({
  description,
}: {
  readonly description?: string;
}) {
  const form = useFormContext<RecipientTargetValues>();
  const targetKind: BillingTargetKind = form.watch("target_kind");
  return (
    <>
      <FormField
        control={form.control}
        name="target_kind"
        render={({ field }) => (
          <FormItem>
            <FormLabel>Recipients</FormLabel>
            <Select
              value={field.value}
              onValueChange={(value) => {
                field.onChange(value);
                if (value !== "selected_users")
                  form.setValue("target_user_ids", []);
                if (value !== "org_members")
                  form.setValue("target_org_ids", []);
                if (value !== "groups") form.setValue("target_group_ids", []);
              }}
            >
              <FormControl>
                <SelectTrigger>
                  <SelectValue />
                </SelectTrigger>
              </FormControl>
              <SelectContent>
                <SelectItem value="all_users">All billing owners</SelectItem>
                <SelectItem value="selected_users">Selected owners</SelectItem>
                <SelectItem value="org_members">
                  Organization members
                </SelectItem>
                <SelectItem value="groups">Group members</SelectItem>
              </SelectContent>
            </Select>
            {description ? (
              <FormDescription className="text-[11px]">
                {description}
              </FormDescription>
            ) : null}
            <FormMessage />
          </FormItem>
        )}
      />
      <p className="text-[11px] text-muted-foreground">
        {targetKind === "org_members"
          ? "Every person in the selected organizations receives the benefit on their personal wallet, including viewers."
          : targetKind === "groups"
            ? "Every person directly in the selected groups receives the benefit on their personal wallet."
            : targetKind === "selected_users"
              ? "Select a person for their personal wallet, or an organization for its shared wallet."
              : "All active person and organization wallets."}
      </p>
      {targetKind === "org_members" ? (
        <FormField
          control={form.control}
          name="target_org_ids"
          render={({ field }) => (
            <FormItem>
              <FormLabel>Organizations</FormLabel>
              <FormControl>
                <OrgPicker
                  selected={field.value ?? []}
                  onChange={field.onChange}
                />
              </FormControl>
              <FormMessage />
            </FormItem>
          )}
        />
      ) : null}
      {targetKind === "groups" ? (
        <FormField
          control={form.control}
          name="target_group_ids"
          render={({ field }) => (
            <FormItem>
              <FormLabel>Groups</FormLabel>
              <FormControl>
                <GroupPicker
                  selected={field.value ?? []}
                  onChange={field.onChange}
                />
              </FormControl>
              <FormMessage />
            </FormItem>
          )}
        />
      ) : null}
      {targetKind === "selected_users" ? (
        <FormField
          control={form.control}
          name="target_user_ids"
          render={({ field }) => (
            <FormItem>
              <FormLabel>Owners</FormLabel>
              <FormControl>
                <UserPicker selected={field.value} onChange={field.onChange} />
              </FormControl>
              <FormMessage />
            </FormItem>
          )}
        />
      ) : null}
    </>
  );
}
