import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";
import {
  parseCrmSavedViewResponse,
  parseCrmSavedViewsResponse,
  parseCrmViewDeleteResponse,
  parseCrmViewValidationResponse,
} from "@/hooks/use-crm-views";
import { crmViewEnvelopeBaseSchema } from "@/lib/crm/views/schema";
import { validateCrmViewEnvelope } from "@/lib/crm/views/validation";

type ViewFixtureName =
  | "views_list"
  | "view_get"
  | "view_create"
  | "view_delete"
  | "view_validate_valid"
  | "view_validate_invalid";

function fixture(name: ViewFixtureName): unknown {
  return JSON.parse(
    readFileSync(
      resolve(__dirname, `../../../docs/fixtures/crm/${name}.json`),
      "utf8",
    ),
  ) as unknown;
}

const companyMetadata = [
  {
    key: "company",
    fields: [
      { key: "name", archived: false },
      { key: "domain", archived: false },
      { key: "is_partner", archived: false },
    ],
  },
] as const;

describe("CRM saved-view route fixtures", () => {
  it("parses every saved-view response through the production hook parsers", () => {
    expect(parseCrmSavedViewsResponse(fixture("views_list"))).toHaveLength(1);
    expect(parseCrmSavedViewResponse(fixture("view_get")).valid).toBe(true);
    expect(parseCrmSavedViewResponse(fixture("view_create")).revision).toBe(1);
    expect(parseCrmViewDeleteResponse(fixture("view_delete"))).toMatchObject({
      deleted: true,
    });
    expect(
      parseCrmViewValidationResponse(fixture("view_validate_valid")),
    ).toEqual({ valid: true, errors: [] });
    expect(
      parseCrmViewValidationResponse(fixture("view_validate_invalid")).valid,
    ).toBe(false);
  });

  it("accepts the fetched envelope when the server marks it valid", () => {
    const view = parseCrmSavedViewResponse(fixture("view_get"));

    expect(view.valid).toBe(true);
    expect(
      validateCrmViewEnvelope(view.envelope, companyMetadata).success,
    ).toBe(true);
  });

  it("matches server and client paths for the same invalid envelope", () => {
    const view = parseCrmSavedViewResponse(fixture("view_get"));
    const invalidEnvelope = crmViewEnvelopeBaseSchema.parse(view.envelope);
    const companyTable = invalidEnvelope.spec.elements.CompanyTable;
    expect(companyTable).toBeDefined();
    if (!companyTable) throw new Error("View fixture is missing CompanyTable");
    companyTable.props.columns = ["unknown_field"];

    const server = parseCrmViewValidationResponse(
      fixture("view_validate_invalid"),
    );
    const client = validateCrmViewEnvelope(invalidEnvelope, companyMetadata);
    expect(client.success).toBe(false);
    if (client.success)
      throw new Error("Invalid view fixture passed validation");

    expect(client.errors.map((error) => error.path)).toEqual(
      server.errors.map((error) => error.path),
    );
  });
});
