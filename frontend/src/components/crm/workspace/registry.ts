import { defineCatalog } from "@json-render/core";
import {
  defineRegistry,
  schema as jsonRenderSchema,
  type Components,
} from "@json-render/react";
import { crmViewComponentCatalog } from "@/lib/crm/views/catalog";

type CatalogDefinitions = {
  [ComponentName in keyof typeof crmViewComponentCatalog]: {
    readonly props: (typeof crmViewComponentCatalog)[ComponentName]["props"];
    readonly description: string;
    readonly slots: string[];
  };
};

const catalogDefinitions = Object.fromEntries(
  Object.entries(crmViewComponentCatalog).map(([name, definition]) => [
    name,
    {
      props: definition.props,
      description: definition.description,
      slots: [...definition.slots],
    },
  ]),
) as CatalogDefinitions;

export const crmJsonRenderCatalog = defineCatalog(jsonRenderSchema, {
  components: catalogDefinitions,
  actions: {},
});

const registryComponents = Object.fromEntries(
  Object.entries(crmViewComponentCatalog).map(([name, definition]) => [
    name,
    definition.component,
  ]),
) as Components<typeof crmJsonRenderCatalog>;

export const { registry: crmViewRegistry } = defineRegistry(
  crmJsonRenderCatalog,
  { components: registryComponents },
);
