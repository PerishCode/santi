//! `runseal :audit` — read-only tool-activity view projected from the local
//! Keel estate. Thin entry point; logic lives in the audit module.

import { audit } from "@/lib/audit/audit.ts";

Deno.exit(await audit(Deno.args));
