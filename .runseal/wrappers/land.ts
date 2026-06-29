//! `runseal :land <title> [--body <text>]`
//!
//! One-shot squash-merge of the current branch's PR. Thin entry point; logic
//! lives in the land module.

import { land } from "@/lib/land/land.ts";

Deno.exit(await land(Deno.args));
