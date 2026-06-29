//! `runseal :init [--force]` — install local git hooks. Thin entry point; logic
//! lives in the init module.

import { init } from "@/lib/init/init.ts";

Deno.exit(await init(Deno.args));
