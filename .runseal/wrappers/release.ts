//! `runseal :release <step>` — one step of the CI release pipeline. Thin entry
//! point; logic lives in the release module.

import { release } from "@/lib/release/release.ts";

Deno.exit(await release(Deno.args));
