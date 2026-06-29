//! `runseal :dev <start|stop|restart|status|logs>`
//!
//! A thin entry point. All process-management logic lives in the manager so the
//! wrapper stays a dispatcher.

import { run } from "@/lib/dev/manager.ts";

await run(Deno.args);
