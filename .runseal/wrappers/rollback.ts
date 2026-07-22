//! `runseal :rollback ...` — inspect or act on an armed recovery capsule.

import { recovery } from "@/lib/recovery/recovery.ts";

Deno.exit(await recovery(Deno.args));
