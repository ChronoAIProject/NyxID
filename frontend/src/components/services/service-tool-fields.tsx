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
  const { data: topics = [] } = useToolTopics();
  const selected = form.watch("topics") ?? [];
  return (
    <div className="space-y-4">
      <FormItem>
        <FormLabel>Offering kind</FormLabel>
        <Select
          value={form.watch("offering_kind")}
          onValueChange={(value) =>
            form.setValue("offering_kind", value as "ai_service" | "tool")
          }
        >
          <SelectTrigger aria-label="Offering kind">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="ai_service">AI Service</SelectItem>
            <SelectItem value="tool">Tool</SelectItem>
          </SelectContent>
        </Select>
      </FormItem>
      <FormField
        control={form.control}
        name="supplier"
        render={({ field }) => (
          <FormItem>
            <FormLabel>Supplier</FormLabel>
            <FormControl>
              <Input {...field} />
            </FormControl>
            <FormMessage />
          </FormItem>
        )}
      />
      <FormItem>
        <FormLabel>Topics</FormLabel>
        <div className="flex flex-wrap gap-2" role="group" aria-label="Topics">
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
        <p className="text-11 text-text-tertiary">
          Import source: {service.import_source.kind} ·{" "}
          {service.import_source.reference} ·{" "}
          {service.import_source.version ?? "Unversioned"} ·{" "}
          {service.import_source.imported_at ?? "Unknown date"}
        </p>
      )}
    </div>
  );
}
