import { init } from "@perish/sealkit/init";

await init({
  tools: ["git", "deno", "cargo", "runseal", "ectropy", "plumb"],
  paths: [
    "Cargo.toml",
    "Cargo.lock",
    "ectropy.toml",
    "runseal.toml",
    "plumb.toml",
    ".forgejo/workflows/guard.yml",
    ".forgejo/workflows/release-exact.yml",
    ".forgejo/workflows/release-stable.yml",
    ".runseal/deno.json",
    ".runseal/deno.lock",
    ".runseal/hooks/pre-commit",
    ".runseal/hooks/commit-msg",
    ".runseal/wrappers/guard.ts",
    ".runseal/wrappers/init.ts",
    ".runseal/wrappers/land.ts",
    ".runseal/lib/deploy/deploy.ts",
  ],
});
