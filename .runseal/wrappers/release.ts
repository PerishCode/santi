//! `runseal :release <step>` — one step of the CI release pipeline. Use
//! `runseal :release --help` for the step contract. Thin entry point; logic
//! lives in the release module.

import { release } from "@/lib/release/release.ts";

Deno.exit(await release(Deno.args));
