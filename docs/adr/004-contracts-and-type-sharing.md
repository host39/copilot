# ADR-004: Contracts and Type Sharing

## Status

Accepted

## Context

In the TypeScript starter, one Zod schema per payload serves both the API (validation, OpenAPI) and the web app (typed clients). With a Rust API, the server's types live in Rust, and the web app still needs runtime validation of responses. Two hand-written copies would drift silently.

## Decision

- **Rust DTOs are the source of truth.** `packages/contracts` holds request and response structs with `serde` (camelCase on the wire) and `utoipa::ToSchema` (OpenAPI).
- **TypeScript types are generated.** `make contracts` runs the contract crate's tests, which export ts-rs bindings to `packages/contracts/ts/generated/`. ts-rs is a dev-dependency, so release builds never compile it. The generated files are committed.
- **Zod schemas are checked against the generated types.** `packages/contracts/ts/index.ts` defines each schema with `satisfies z.ZodType<GeneratedType>`, so a mismatch fails `make type-check`.
- **CI rejects stale bindings**: it regenerates them and fails on any diff or untracked file.
- The web app imports only `@copilot/contracts`; core never depends on contracts. API handlers map entities to DTOs.

## Consequences

### Positive

- One definition per payload; drift between Rust, OpenAPI, TypeScript types, and Zod is caught at build time.
- The web app keeps runtime validation of everything it receives.

### Negative

- Contract changes need a regeneration step (`make contracts`) and a Zod update.
- `satisfies` proves the schema output fits the generated type; an extra field in Zod that Rust does not send is not caught.

### Neutral

- 64-bit integers are exported as `number` (`TS_RS_LARGE_INT`); payloads that need values beyond 2^53 should use strings.

## Alternatives Considered

| Alternative | Why rejected |
| --- | --- |
| Hand-written Zod only | No link to the Rust types; drift appears only at runtime |
| Generate a client from the OpenAPI document | Adds a code generator and a second source of types for the web app |
| Generated types without Zod | No runtime validation in the browser |

## References

- [ADR-001: Domain-Driven Design](./001-domain-driven-design.md)
- [ADR-003: Rust Backend Stack](./003-rust-backend-stack.md)
- [ts-rs](https://docs.rs/ts-rs)
