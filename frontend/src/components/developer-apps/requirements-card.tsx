import { useState } from "react";
import { useFieldArray } from "react-hook-form";
import { zodResolver } from "@hookform/resolvers/zod";
import { toast } from "sonner";
import {
  useAppRequirementManifests,
  usePublishAppRequirements,
} from "@/hooks/use-app-requirements";
import { useCatalog } from "@/hooks/use-keys";
import {
  publishManifestSchema,
  type PublishManifest,
  type ServiceRequirement,
} from "@/schemas/app-requirements";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Form, FormSubmitErrors, useAppForm } from "@/components/ui/form";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Checkbox } from "@/components/ui/checkbox";
import { Badge } from "@/components/ui/badge";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";

function newRequirement(): ServiceRequirement {
  return {
    id: "",
    label: "",
    any_of_catalog_slugs: [],
    any_of_catalog_prefix: null,
    owner_policy: "personal_only",
    accepted_credential_types: [],
    allow_master_credential: false,
    allow_no_credential: false,
    required_downstream_scopes: [],
    validator: { kind: "stored_only" },
    optional: false,
  };
}

const credentialTypes = [
  "api_key",
  "oauth2",
  "bearer",
  "basic",
  "node_managed",
  "ssh_certificate",
  "aws_sigv4",
  "gcp_service_account",
];

export function RequirementsCard({ clientId }: { readonly clientId: string }) {
  const { data, isPending, error } = useAppRequirementManifests(clientId);
  const { data: catalog = [] } = useCatalog({ includeAll: true });
  const publish = usePublishAppRequirements(clientId);
  const [editing, setEditing] = useState(false);
  const form = useAppForm<PublishManifest>({
    resolver: zodResolver(publishManifestSchema),
    defaultValues: { enforcement: "advise", requirements: [] },
  });
  const { fields, append, remove } = useFieldArray({
    control: form.control,
    name: "requirements",
    keyName: "fieldKey",
  });
  const requirements = form.watch("requirements");

  function startPublish() {
    form.reset({
      enforcement: "advise",
      requirements: data?.versions[0]?.requirements ?? [],
    });
    setEditing(true);
  }

  async function submit(input: PublishManifest) {
    try {
      await publish.mutateAsync({
        ...input,
        requirements: input.requirements.map((r) => ({
          ...r,
          required_downstream_scopes: [
            ...new Set(r.required_downstream_scopes.filter(Boolean)),
          ],
        })),
      });
      setEditing(false);
      toast.success("Requirements version published");
    } catch (err) {
      toast.error(
        err instanceof Error ? err.message : "Could not publish requirements",
      );
    }
  }

  return (
    <Card>
      <CardHeader>
        <CardTitle>Requirements</CardTitle>
        <CardDescription>
          Declare the services your app needs. Each published version keeps its
          catalog selection. Readiness is advisory and does not change a user’s
          access grant.
        </CardDescription>
      </CardHeader>
      <CardContent className="space-y-4">
        {isPending && (
          <p className="text-sm text-muted-foreground">Loading requirements…</p>
        )}
        {error && (
          <p role="alert" className="text-sm text-destructive">
            {error.message}
          </p>
        )}
        {data && (
          <>
            {data.versions.length === 0 && (
              <p className="text-sm text-muted-foreground">
                No requirements published.
              </p>
            )}
            <div className="space-y-2">
              {data.versions.map((version) => (
                <details key={version.id} className="rounded-lg border p-3">
                  <summary className="cursor-pointer text-sm">
                    Version {version.version} · {version.requirements.length}{" "}
                    requirements ·{" "}
                    {new Date(version.published_at).toLocaleDateString()}{" "}
                    <Badge variant="secondary">{version.enforcement}</Badge>
                  </summary>
                  <ul className="mt-2 space-y-2 text-sm">
                    {version.requirements.map((r) => (
                      <li key={r.id}>
                        <span className="font-medium">{r.label}</span>
                        <span className="text-muted-foreground">
                          {" "}
                          · {r.any_of_catalog_slugs.join(", ")} ·{" "}
                          {r.validator.kind === "profile"
                            ? r.validator.id
                            : "Stored credential"}
                          {r.optional ? " · Optional" : ""}
                        </span>
                      </li>
                    ))}
                  </ul>
                </details>
              ))}
            </div>
            {!editing && (
              <Button type="button" variant="outline" onClick={startPublish}>
                Publish new version
              </Button>
            )}
          </>
        )}
        {editing && (
          <Form {...form}>
            <form onSubmit={form.handleSubmit(submit)} className="space-y-4">
              <div className="space-y-2">
                <Label>Enforcement</Label>
                <Badge variant="secondary">Advise</Badge>
                <p className="text-xs text-muted-foreground">
                  Users can sign in while requirements are unmet. Gate
                  enforcement is not available yet.
                </p>
              </div>
              {fields.map((field, index) => {
                const value = requirements[index]!;
                const path = `requirements.${index}` as const;
                return (
                  <fieldset
                    key={field.fieldKey}
                    className="space-y-3 rounded-lg border p-4"
                  >
                    <legend className="px-1 text-sm font-medium">
                      Requirement {index + 1}
                    </legend>
                    <div className="grid gap-3 sm:grid-cols-2">
                      <div className="space-y-1">
                        <Label htmlFor={`${field.fieldKey}-id`}>ID</Label>
                        <Input
                          id={`${field.fieldKey}-id`}
                          {...form.register(`${path}.id`)}
                          placeholder="github"
                        />
                      </div>
                      <div className="space-y-1">
                        <Label htmlFor={`${field.fieldKey}-label`}>Label</Label>
                        <Input
                          id={`${field.fieldKey}-label`}
                          {...form.register(`${path}.label`)}
                          placeholder="Source code"
                        />
                      </div>
                    </div>
                    <div className="space-y-2">
                      <Label>Any of these catalog services</Label>
                      <div className="max-h-40 space-y-2 overflow-y-auto rounded-lg border p-3">
                        {catalog.map((c) => (
                          <label
                            key={c.slug}
                            className="flex items-center gap-2 text-sm"
                          >
                            <Checkbox
                              checked={value.any_of_catalog_slugs.includes(
                                c.slug,
                              )}
                              onCheckedChange={(checked) =>
                                form.setValue(
                                  `${path}.any_of_catalog_slugs`,
                                  checked
                                    ? [...value.any_of_catalog_slugs, c.slug]
                                    : value.any_of_catalog_slugs.filter(
                                        (slug) => slug !== c.slug,
                                      ),
                                )
                              }
                            />
                            {c.name}
                            <span className="text-xs text-muted-foreground">
                              {c.slug}
                            </span>
                          </label>
                        ))}
                      </div>
                    </div>
                    <div className="space-y-1">
                      <Label htmlFor={`${field.fieldKey}-prefix`}>
                        Catalog prefix (optional)
                      </Label>
                      <Input
                        id={`${field.fieldKey}-prefix`}
                        placeholder="llm-"
                        value={value.any_of_catalog_prefix ?? ""}
                        onChange={(e) =>
                          form.setValue(
                            `${path}.any_of_catalog_prefix`,
                            e.target.value || null,
                          )
                        }
                      />
                      <p className="text-xs text-muted-foreground">
                        Adds active seeded services matching this prefix when
                        you publish.
                      </p>
                    </div>
                    <div className="space-y-1">
                      <Label>Ownership</Label>
                      <Select
                        value={value.owner_policy}
                        onValueChange={(
                          policy: ServiceRequirement["owner_policy"],
                        ) => form.setValue(`${path}.owner_policy`, policy)}
                      >
                        <SelectTrigger
                          aria-label={`Ownership for requirement ${index + 1}`}
                        >
                          <SelectValue />
                        </SelectTrigger>
                        <SelectContent>
                          <SelectItem value="personal_only">
                            Personal only
                          </SelectItem>
                          <SelectItem value="personal_or_org_allowed">
                            Personal or permitted organization
                          </SelectItem>
                        </SelectContent>
                      </Select>
                    </div>
                    <div className="space-y-2">
                      <Label>Accepted credential types</Label>
                      <p className="text-xs text-muted-foreground">
                        Leave all unchecked to accept any user credential.
                      </p>
                      <div className="flex flex-wrap gap-3">
                        {credentialTypes.map((type) => (
                          <label
                            key={type}
                            className="flex items-center gap-2 text-sm"
                          >
                            <Checkbox
                              checked={value.accepted_credential_types.includes(
                                type,
                              )}
                              onCheckedChange={(checked) =>
                                form.setValue(
                                  `${path}.accepted_credential_types`,
                                  checked
                                    ? [...value.accepted_credential_types, type]
                                    : value.accepted_credential_types.filter(
                                        (t) => t !== type,
                                      ),
                                )
                              }
                            />
                            {type}
                          </label>
                        ))}
                      </div>
                    </div>
                    <div className="space-y-1">
                      <Label htmlFor={`${field.fieldKey}-scopes`}>
                        Required downstream OAuth scopes
                      </Label>
                      <Input
                        id={`${field.fieldKey}-scopes`}
                        placeholder="repo read:user"
                        value={value.required_downstream_scopes.join(" ")}
                        onChange={(e) =>
                          form.setValue(
                            `${path}.required_downstream_scopes`,
                            e.target.value.split(" "),
                          )
                        }
                      />
                    </div>
                    <div className="space-y-1">
                      <Label>Validation</Label>
                      <Select
                        value={
                          value.validator.kind === "profile"
                            ? value.validator.id
                            : "stored_only"
                        }
                        onValueChange={(id) =>
                          form.setValue(
                            `${path}.validator`,
                            id === "stored_only"
                              ? { kind: "stored_only" }
                              : { kind: "profile", id },
                          )
                        }
                      >
                        <SelectTrigger
                          aria-label={`Validation for requirement ${index + 1}`}
                        >
                          <SelectValue />
                        </SelectTrigger>
                        <SelectContent>
                          <SelectItem value="stored_only">
                            Stored credential only
                          </SelectItem>
                          {data?.validator_profiles.map((profile) => (
                            <SelectItem key={profile.id} value={profile.id}>
                              {profile.id}
                            </SelectItem>
                          ))}
                        </SelectContent>
                      </Select>
                      {value.validator.kind === "profile" && (
                        <p className="text-xs text-muted-foreground">
                          {
                            data?.validator_profiles.find(
                              (p) =>
                                value.validator.kind === "profile" &&
                                p.id === value.validator.id,
                            )?.claim
                          }
                        </p>
                      )}
                    </div>
                    <div className="flex flex-wrap gap-4">
                      {(
                        [
                          [
                            "allow_master_credential",
                            "Allow platform credential",
                          ],
                          [
                            "allow_no_credential",
                            "Allow credential-free service",
                          ],
                          ["optional", "Optional"],
                        ] as const
                      ).map(([key, label]) => (
                        <label
                          key={key}
                          className="flex items-center gap-2 text-sm"
                        >
                          <Checkbox
                            checked={value[key]}
                            onCheckedChange={(checked) =>
                              form.setValue(`${path}.${key}`, checked === true)
                            }
                          />
                          {label}
                        </label>
                      ))}
                    </div>
                    <Button
                      type="button"
                      size="sm"
                      variant="ghost"
                      onClick={() => remove(index)}
                    >
                      Remove requirement
                    </Button>
                  </fieldset>
                );
              })}
              <Button
                type="button"
                variant="outline"
                disabled={fields.length >= 25}
                onClick={() => append(newRequirement())}
              >
                Add requirement
              </Button>
              <div className="flex items-center gap-2">
                <Button type="submit" disabled={publish.isPending}>
                  {publish.isPending ? "Publishing…" : "Publish version"}
                </Button>
                <Button
                  type="button"
                  variant="ghost"
                  onClick={() => setEditing(false)}
                  disabled={publish.isPending}
                >
                  Cancel
                </Button>
              </div>
              <FormSubmitErrors />
            </form>
          </Form>
        )}
      </CardContent>
    </Card>
  );
}
