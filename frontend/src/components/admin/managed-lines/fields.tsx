import type { Control, FieldPath, FieldValues } from "react-hook-form";
import { translateUi, useUiTranslation } from "@/i18n/use-ui-translation";
import { useServices } from "@/hooks/use-services";
import {
  FormControl,
  FormField,
  FormItem,
  FormLabel,
  FormMessage,
} from "@/components/ui/form";
import { Input } from "@/components/ui/input";
import { Textarea } from "@/components/ui/textarea";
import { Checkbox } from "@/components/ui/checkbox";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";

type FieldProps<T extends FieldValues> = {
  readonly control: Control<T>;
  readonly name: FieldPath<T>;
  readonly label: string;
  readonly help?: string;
};

export function TextField<T extends FieldValues>({
  control,
  name,
  label,
  help,
  type,
  multiline,
}: FieldProps<T> & { readonly type?: "text" | "password"; readonly multiline?: boolean }) {
  return (
    <FormField
      control={control}
      name={name}
      render={({ field }) => (
        <FormItem>
          <FormLabel>{label}</FormLabel>
          <FormControl>
            {multiline ? (
              <Textarea {...field} value={field.value ?? ""} rows={4} className="font-mono" />
            ) : (
              <Input {...field} value={field.value ?? ""} type={type} autoComplete="off" />
            )}
          </FormControl>
          {help && <p className="type-secondary text-text-tertiary">{help}</p>}
          <FormMessage />
        </FormItem>
      )}
    />
  );
}

export function NumberField<T extends FieldValues>({ control, name, label, help }: FieldProps<T>) {
  return (
    <FormField
      control={control}
      name={name}
      render={({ field }) => (
        <FormItem>
          <FormLabel>{label}</FormLabel>
          <FormControl>
            <Input
              type="number"
              inputMode="numeric"
              value={Number.isFinite(field.value) ? field.value : ""}
              onChange={(event) => field.onChange(event.target.valueAsNumber)}
              onBlur={field.onBlur}
              name={field.name}
              ref={field.ref}
            />
          </FormControl>
          {help && <p className="type-secondary text-text-tertiary">{help}</p>}
          <FormMessage />
        </FormItem>
      )}
    />
  );
}

export function CheckField<T extends FieldValues>({ control, name, label, help }: FieldProps<T>) {
  return (
    <FormField
      control={control}
      name={name}
      render={({ field }) => (
        <FormItem>
          <Label className="flex items-start gap-2">
            <Checkbox
              checked={field.value === true}
              onCheckedChange={(value) => field.onChange(value === true)}
            />
            <span>{label}</span>
          </Label>
          {help && <p className="type-secondary text-text-tertiary">{help}</p>}
          <FormMessage />
        </FormItem>
      )}
    />
  );
}

export function ChoiceField<T extends FieldValues>({
  control,
  name,
  label,
  help,
  choices,
  placeholder,
}: FieldProps<T> & {
  readonly choices: readonly { readonly value: string; readonly label: string }[];
  readonly placeholder?: string;
}) {
  return (
    <FormField
      control={control}
      name={name}
      render={({ field }) => (
        <FormItem>
          <FormLabel>{label}</FormLabel>
          <Select value={field.value || undefined} onValueChange={field.onChange}>
            <FormControl>
              <SelectTrigger>
                <SelectValue placeholder={placeholder} />
              </SelectTrigger>
            </FormControl>
            <SelectContent>
              {choices.map((c) => (
                <SelectItem key={c.value} value={c.value}>
                  {c.label}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
          {help && <p className="type-secondary text-text-tertiary">{help}</p>}
          <FormMessage />
        </FormItem>
      )}
    />
  );
}

/** Dedicated managed billing services are private internal catalog rows. */
export function ServiceField<T extends FieldValues>(props: FieldProps<T>) {
  useUiTranslation();
  const services = useServices(true);
  return (
    <ChoiceField
      {...props}
      placeholder={translateUi("Choose a dedicated private service")}
      choices={(services.data ?? [])
        .filter((s) => s.visibility === "private" && s.service_category === "internal")
        .map((s) => ({ value: s.id, label: `${s.name} · ${s.slug}` }))}
    />
  );
}
