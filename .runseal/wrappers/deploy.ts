//! `runseal :deploy <version>` — deploy one exact santi beta to its live host. Thin
//! entry point; logic lives in the deploy module.

import { deploy } from "@/lib/deploy/deploy.ts";

Deno.exit(await deploy(Deno.args));
