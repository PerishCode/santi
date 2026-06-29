//! `runseal :guard` — run the local validation suite (mirrors CI smoke + Deno
//! checks). Thin entry point; logic lives in the guard module.

import { guard } from "@/lib/guard/guard.ts";

Deno.exit(await guard());
