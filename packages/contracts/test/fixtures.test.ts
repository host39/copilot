// Contract conformance, TypeScript side. `tests/fixtures.rs` writes sample JSON from the Rust
// structs for every generated contract type (`test/fixtures/types/<Type>.json`) and every route
// body (`test/fixtures/bodies/<name>.json`, parsed with `<name>BodySchema`). Each sample must
// parse with its Zod schema and come back unchanged (a field the schema does not know would be
// dropped), and the samples together must set every field, enum value, and union variant the
// schema declares. A new contract type or route body fails here until it has samples: run
// `make contracts` and commit the fixtures with the bindings.

import { readdirSync, readFileSync } from "node:fs";
import * as contracts from "@contracts/index";
import { describe, expect, test } from "@rstest/core";
import * as auth from "@rustauth/contracts";
import { z } from "zod";

const FIXTURES = "test/fixtures";

/** Generic envelopes have no schema of their own; the route bodies cover them. */
const ENVELOPES = new Set(["DetailsResponse", "ListResponse", "PageResponse"]);

/** Generated types whose schema is not named `<camelCaseType>Schema`. */
const SCHEMA_NAMES: Record<string, string> = {
  MessagePartDto: "messagePartSchema",
  OAuthClientInput: "oauthClientInputSchema",
  OAuthStartResponse: "oauthStartResponseSchema",
  OAuthStatus: "oauthStatusSchema",
};

const BODY = "BodySchema";

const exported = contracts as Record<string, unknown>;

/** File names in `folder` that end with `extension`, without it, sorted. */
function names(folder: string, extension: string): string[] {
  return readdirSync(folder, { withFileTypes: true })
    .filter((entry) => entry.isFile() && entry.name.endsWith(extension))
    .map((entry) => entry.name.slice(0, -extension.length))
    .sort();
}

/** Every generated contract type except the envelopes. */
const TYPES = names("ts/generated", ".ts").filter((name) => !ENVELOPES.has(name));

/** Every route body schema of `@copilot/contracts` (rustauth's are tested in lib-rustauth). */
const BODIES = Object.keys(contracts)
  .filter((name) => name.endsWith(BODY) && !(name in auth))
  .map((name) => name.slice(0, -BODY.length))
  .sort();

function samples(folder: string, name: string): unknown[] {
  const value: unknown = JSON.parse(readFileSync(`${FIXTURES}/${folder}/${name}.json`, "utf8"));
  if (!Array.isArray(value) || value.length === 0) {
    throw new Error(`${folder}/${name}.json must hold a non-empty array of samples`);
  }
  return value;
}

function schema(name: string): z.ZodType {
  const found = exported[name];
  if (!(found instanceof z.ZodType)) {
    throw new Error(`@copilot/contracts exports no schema named ${name}`);
  }
  return found;
}

function typeSchema(type: string): z.ZodType {
  return schema(SCHEMA_NAMES[type] ?? `${type.charAt(0).toLowerCase()}${type.slice(1)}Schema`);
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

/**
 * Paths where `parsed` lost or changed a value of `sample`. Keys only the parse adds (Zod
 * defaults for fields the server may leave out) are fine.
 */
function changed(sample: unknown, parsed: unknown, path = "$"): string[] {
  if (Array.isArray(sample)) {
    if (!Array.isArray(parsed) || parsed.length !== sample.length) return [path];
    return sample.flatMap((item, index) => changed(item, parsed[index], `${path}[${index}]`));
  }
  if (isRecord(sample)) {
    if (!isRecord(parsed)) return [path];
    return Object.entries(sample).flatMap(([key, value]) =>
      key in parsed ? changed(value, parsed[key], `${path}.${key}`) : [`${path}.${key}`]
    );
  }
  return Object.is(sample, parsed) ? [] : [path];
}

/** Fields of `shape` that none of `values` sets. */
function unsetFields(shape: Record<string, unknown>, values: unknown[]): string[] {
  const seen = new Set(values.flatMap((value) => (isRecord(value) ? Object.keys(value) : [])));
  return Object.keys(shape).filter((key) => !seen.has(key));
}

/** Union variants with no sample, and fields of a variant that none of its samples sets. */
function unsetVariants(union: z.ZodDiscriminatedUnion, values: unknown[]): string[] {
  const key = union.def.discriminator;
  return union.options.flatMap((option) => {
    if (!(option instanceof z.ZodObject)) return ["(variant without fields)"];
    const tag = option.shape[key];
    const tags: unknown[] = tag instanceof z.ZodLiteral ? [...tag.values] : [];
    const own = values.filter((value) => isRecord(value) && tags.includes(value[key]));
    if (own.length === 0) return tags.map(String);
    return unsetFields(option.shape, own).map((field) => `${tags.join("|")}.${field}`);
  });
}

/** What `schema` declares that no sample reaches: enum values, union variants, or fields. */
function unreached(schema: z.ZodType, values: unknown[]): string[] {
  if (schema instanceof z.ZodEnum) {
    return schema.options.filter((option) => !values.includes(option)).map(String);
  }
  if (schema instanceof z.ZodDiscriminatedUnion) return unsetVariants(schema, values);
  if (schema instanceof z.ZodObject) return unsetFields(schema.shape, values);
  return [];
}

function conforms(schema: z.ZodType, values: unknown[]) {
  values.forEach((sample, index) => {
    const result = schema.safeParse(sample);
    expect({ index, issues: result.error?.issues ?? [] }).toEqual({ index, issues: [] });
    expect({ index, changed: changed(sample, result.data) }).toEqual({ index, changed: [] });
  });
  expect(unreached(schema, values)).toEqual([]);
}

describe("contract fixtures", () => {
  test("cover every generated type", () => {
    expect(names(`${FIXTURES}/types`, ".json")).toEqual(TYPES);
  });

  test("cover every route body", () => {
    expect(BODIES.length).toBeGreaterThan(0);
    expect(names(`${FIXTURES}/bodies`, ".json")).toEqual(BODIES);
  });

  test("catch what a schema drops, changes, or never sees", () => {
    const shape = z.object({ id: z.string(), tags: z.array(z.string()).default([]) });
    const parsed = shape.parse({ id: "a", extra: 1 });
    expect(changed({ id: "a", extra: 1 }, parsed)).toEqual(["$.extra"]);
    expect(changed({ id: "a" }, parsed)).toEqual([]);
    expect(changed({ id: ["a"] }, { id: "a" })).toEqual(["$.id"]);
    expect(unreached(shape, [{ id: "a" }])).toEqual(["tags"]);
    expect(unreached(z.enum(["on", "off"]), ["on"])).toEqual(["off"]);
    const union = z.discriminatedUnion("type", [
      z.object({ type: z.literal("a"), value: z.number() }),
      z.object({ type: z.literal("b") }),
    ]);
    expect(unreached(union, [{ type: "a" }])).toEqual(["a.value", "b"]);
  });
});

describe("contract types parse with their schemas", () => {
  for (const type of TYPES) {
    test(type, () => {
      conforms(typeSchema(type), samples("types", type));
    });
  }
});

describe("route bodies parse with their schemas", () => {
  for (const body of BODIES) {
    test(body, () => {
      conforms(schema(`${body}${BODY}`), samples("bodies", body));
    });
  }
});
