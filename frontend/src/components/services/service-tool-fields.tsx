import { useUiTranslation } from "@/i18n/use-ui-translation";
import type { UseFormReturn } from "react-hook-form";
import type { UpdateServiceFormData } from "@/schemas/services";
import type { DownstreamService } from "@/types/api";
import { useToolTopics } from "@/hooks/use-tools";
import {
  FormControl,
  FormField,
  FormItem,
  FormLabel,
  FormMessage,
} from "@/components/ui/form";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";

export function ServiceToolFields({
  form,
  service,
}: {
  form: UseFormReturn<UpdateServiceFormData>;
  service: DownstreamService;
}) {
  const { t } = useUiTranslation();
  const { data: topics = [] } = useToolTopics();
  const selected = form.watch("topics") ?? [];
  return (
    <div className="space-y-4">
      <FormItem>
        <FormLabel>{t("Offering kind")}</FormLabel>
        <Select
          value={form.watch("offering_kind")}
          onValueChange={(value) =>
            form.setValue("offering_kind", value as "ai_service" | "tool")
          }
        >
          <SelectTrigger aria-label={t("Offering kind")}>
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="ai_service">{t("AI Service")}</SelectItem>
            <SelectItem value="tool">{t("Tool")}</SelectItem>
          </SelectContent>
        </Select>
      </FormItem>
      <FormField
        control={form.control}
        name="supplier"
        render={({ field }) => (
          <FormItem>
            <FormLabel>{t("Supplier")}</FormLabel>
            <FormControl>
              <Input {...field} />
            </FormControl>
            <FormMessage />
          </FormItem>
        )}
      />
      <FormItem>
        <FormLabel>{t("Topics")}</FormLabel>
        <div className="flex flex-wrap gap-2" role="group" aria-label={t("Topics")}>
          {topics.map((topic) => (
            <Button
              key={topic.slug}
              type="button"
              variant={selected.includes(topic.slug) ? "secondary" : "ghost"}
              aria-pressed={selected.includes(topic.slug)}
              onClick={() =>
                form.setValue(
                  "topics",
                  selected.includes(topic.slug)
                    ? selected.filter((value) => value !== topic.slug)
                    : [...selected, topic.slug],
                )
              }
            >
              {topic.label}
            </Button>
          ))}
        </div>
      </FormItem>
      {service.import_source && (
        <p className="text-11 text-text-tertiary">{t("Import source: ")}{service.import_source.kind} ·{" "}
          {service.import_source.reference} ·{" "}
          {service.import_source.version ?? t("Unversioned")} ·{" "}
          {service.import_source.imported_at ?? t("Unknown date")}
        </p>
      )}
    </div>
  );
}
