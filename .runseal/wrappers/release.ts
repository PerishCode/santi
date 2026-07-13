//! `runseal :release --channel beta|stable [options]` — dispatch Forgejo CI.

import { dispatch } from "@/lib/release/dispatch.ts";

Deno.exit(await dispatch(Deno.args));
