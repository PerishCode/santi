import { init } from "@perish/sealkit/init";

await init({
  tools: ["git", "deno", "cargo", "runseal", "ectropy", "plumb"],
  paths: [
    "Cargo.toml",
    "Cargo.lock",
    "ectropy.toml",
    "runseal.toml",
    ".runseal/deno.json",
    ".runseal/deno.lock",
    ".runseal/hooks/pre-commit",
    ".runseal/hooks/commit-msg",
    ".runseal/wrappers/guard.ts",
    ".runseal/wrappers/init.ts",
    ".runseal/wrappers/land.ts",
  ],
});
