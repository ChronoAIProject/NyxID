import { z } from "zod";
import { zodResolver } from "@hookform/resolvers/zod";
import { routeSchema, type AdminModel } from "@/schemas/platform-models";
import {
  adminModelPath,
  usePlatformModelChange,
} from "@/hooks/use-platform-models";
import { Form, useAppForm } from "@/components/ui/form";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { NativeSelect } from "@/components/ui/native-select";
import { Checkbox } from "@/components/ui/checkbox";
import { Label } from "@/components/ui/label";
import { EditErrors, Field, SaveError } from "./shared";

const schema = z.object({ routes: z.array(routeSchema).max(32) });
type Route = z.infer<typeof routeSchema>;
export function RoutesEditor({
  model,
  readOnly,
  reload,
}: {
  readonly model: AdminModel;
  readonly readOnly: boolean;
  readonly reload: () => void;
}) {
  const form = useAppForm<z.infer<typeof schema>>({
    resolver: zodResolver(schema),
    defaultValues: { routes: model.routes },
    mode: "onChange",
  });
  const change = usePlatformModelChange();
  const routes = form.watch("routes");
  function set(index: number, patch: Partial<Route>) {
    form.setValue(
      "routes",
      routes.map((row, i) => (i === index ? { ...row, ...patch } : row)),
    );
  }
  return (
    <Form {...form}>
      <form
        className="space-y-5"
        onSubmit={form.handleSubmit(async (values) => {
          try {
            await change.mutateAsync({
              path: `${adminModelPath(model.id)}/routes`,
              body: { expected_revision: model.revision, ...values },
            });
          } catch {
            /* shown below */
          }
        })}
      >
        <p className="type-body text-muted-foreground">
          Private routing configuration. Users see only the public model and
          retail prices. Every route must serve the same model. Credentials are
          managed in the service catalog; opening this page sends no provider
          request.
        </p>
        <fieldset disabled={readOnly || change.isPending} className="space-y-4">
          {routes.map((route, i) => (
            <div
              key={route.id}
              className="space-y-4 rounded-xl border border-border/50 p-4"
            >
              <div className="flex items-center justify-between gap-3">
                <h4 className="type-card-title">Route {i + 1}</h4>
                {!readOnly && (
                  <Button
                    type="button"
                    variant="ghost"
                    onClick={() =>
                      form.setValue(
                        "routes",
                        routes.filter((_, n) => i !== n),
                      )
                    }
                  >
                    Remove route {i + 1}
                  </Button>
                )}
              </div>
              <Field title={`Route ${i + 1} kind`}>
                <NativeSelect
                  value={route.target.kind}
                  onChange={(e) => {
                    const native_model = route.target.native_model;
                    const target: Route["target"] =
                      e.target.value === "direct_provider"
                        ? {
                            kind: "direct_provider",
                            provider: "openai",
                            native_model,
                          }
                        : e.target.value === "vertex_publisher"
                          ? {
                              kind: "vertex_publisher",
                              publisher: "google",
                              project: "",
                              location: "",
                              native_model,
                            }
                          : {
                              kind: "vertex_openapi_maas",
                              publisher: "",
                              project: "",
                              location: "",
                              native_model,
                            };
                    set(i, { target });
                  }}
                >
                  <option value="vertex_publisher">
                    Vertex publisher (Gemini / Claude)
                  </option>
                  <option value="vertex_openapi_maas">Vertex open MaaS</option>
                  <option value="direct_provider">
                    Direct OpenAI / Anthropic
                  </option>
                </NativeSelect>
              </Field>
              <div className="grid gap-4 sm:grid-cols-2">
                <Field title="Credential catalog-service ID">
                  <Input
                    value={route.credential_service_id}
                    onChange={(e) =>
                      set(i, { credential_service_id: e.target.value })
                    }
                  />
                </Field>
                <Field title="Native model">
                  <Input
                    value={route.target.native_model}
                    onChange={(e) =>
                      set(i, {
                        target: {
                          ...route.target,
                          native_model: e.target.value,
                        },
                      })
                    }
                  />
                </Field>
                {route.target.kind === "direct_provider" ? (
                  <Field title="Provider">
                    <NativeSelect
                      value={route.target.provider}
                      onChange={(e) =>
                        set(i, {
                          target: {
                            kind: "direct_provider",
                            provider: e.target.value as "openai" | "anthropic",
                            native_model: route.target.native_model,
                          },
                        })
                      }
                    >
                      <option value="openai">OpenAI</option>
                      <option value="anthropic">Anthropic</option>
                    </NativeSelect>
                  </Field>
                ) : (
                  <>
                    <Field title="Project">
                      <Input
                        value={route.target.project}
                        onChange={(e) =>
                          route.target.kind !== "direct_provider" &&
                          set(i, {
                            target: {
                              ...route.target,
                              project: e.target.value,
                            },
                          })
                        }
                      />
                    </Field>
                    <Field title="Location">
                      <Input
                        value={route.target.location}
                        onChange={(e) =>
                          route.target.kind !== "direct_provider" &&
                          set(i, {
                            target: {
                              ...route.target,
                              location: e.target.value,
                            },
                          })
                        }
                      />
                    </Field>
                    <Field title="Publisher">
                      {route.target.kind === "vertex_publisher" ? (
                        <NativeSelect
                          value={route.target.publisher}
                          onChange={(e) =>
                            route.target.kind === "vertex_publisher" &&
                            set(i, {
                              target: {
                                ...route.target,
                                publisher: e.target.value as
                                  | "google"
                                  | "anthropic",
                              },
                            })
                          }
                        >
                          <option value="google">Google</option>
                          <option value="anthropic">Anthropic</option>
                        </NativeSelect>
                      ) : (
                        <Input
                          value={route.target.publisher}
                          onChange={(e) =>
                            route.target.kind === "vertex_openapi_maas" &&
                            set(i, {
                              target: {
                                ...route.target,
                                publisher: e.target.value,
                              },
                            })
                          }
                        />
                      )}
                    </Field>
                  </>
                )}
                <Field title="Priority">
                  <Input
                    type="number"
                    min="0"
                    max="65535"
                    value={route.priority}
                    onChange={(e) =>
                      set(i, { priority: e.target.valueAsNumber })
                    }
                  />
                </Field>
                <Field title="Weight">
                  <Input
                    type="number"
                    min="1"
                    max="65535"
                    value={route.weight}
                    onChange={(e) => set(i, { weight: e.target.valueAsNumber })}
                  />
                </Field>
              </div>
              <Label className="flex items-center gap-2">
                <Checkbox
                  checked={route.enabled}
                  onCheckedChange={(checked) =>
                    set(i, { enabled: checked === true })
                  }
                />
                Route enabled
              </Label>
            </div>
          ))}
          {!readOnly && (
            <Button
              type="button"
              variant="outline"
              disabled={routes.length >= 32}
              onClick={() =>
                form.setValue("routes", [
                  ...routes,
                  {
                    id: crypto.randomUUID(),
                    credential_service_id: "",
                    enabled: false,
                    priority: 0,
                    weight: 1,
                    target: {
                      kind: "direct_provider",
                      provider:
                        model.family === "claude" ? "anthropic" : "openai",
                      native_model: model.public_id.replace(/^nyxid\//, ""),
                    },
                  },
                ])
              }
            >
              Add private route
            </Button>
          )}
        </fieldset>
        <SaveError error={change.error} reload={reload} />
        <EditErrors />
        {!readOnly && (
          <div className="flex justify-end gap-2">
            <Button
              type="button"
              variant="outline"
              onClick={() => form.reset({ routes: model.routes })}
            >
              Cancel
            </Button>
            <Button
              variant="primary"
              disabled={
                !form.formState.isDirty ||
                !form.formState.isValid ||
                change.isPending
              }
            >
              Publish route revision
            </Button>
          </div>
        )}
      </form>
    </Form>
  );
}
