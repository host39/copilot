# Import Paths (TypeScript)

Never use `./` or `../` in imports or re-exports. Biome enforces this.

**Cross-package**: use the published name `@copilot/<package>` (for example `@copilot/contracts`).

**Within-package**: use `@<short-name>/<path-from-src>`: `@web/`, `@contracts/`.

```typescript
// BAD
import { apiFetch } from "./client";
import { healthResponseSchema } from "../../../packages/contracts/ts/index";

// GOOD
import { apiFetch } from "@web/api/client";
import { healthResponseSchema } from "@copilot/contracts";
import { fetchHealth } from "@web/api/routes/health";
```

New packages: add `"@<short-name>/*"` to the **root** `tsconfig.json` `paths`, plus an alias in `apps/web/rsbuild.config.ts` and `apps/web/tsconfig.json` when the web app needs it. Cross-package imports use workspace `dependencies` and `package.json` `exports`, not tsconfig paths alone. Does not apply to `package.json` exports, `node_modules`, or `packages/contracts/ts/generated/` (ts-rs writes those files with relative imports and Biome ignores the folder).

Rust has its own rule: `rust-imports`.

## No Inline Import Type Queries

Do not use TypeScript import type queries (`import("module").Type` or `typeof import("module")`) in type positions. Import types at the top of the file instead. Indexed access on imported types (for example `HealthResponse["status"]`) is fine.

```typescript
// BAD
type Props = {
  health: import("@copilot/contracts").HealthResponse;
};

// GOOD
import type { HealthResponse } from "@copilot/contracts";

type Props = {
  health: HealthResponse;
};
```
