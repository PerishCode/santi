//! Internal release step entrypoint used only by Forgejo workflows.

import { release } from "@/lib/release/release.ts";

Deno.exit(await release(Deno.args));
