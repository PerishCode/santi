import { guard } from "@perish/sealkit/guard";
import { assertDebLinger, assertWebhookBoundary } from "@perish/sealkit/operator";
import ingress from "../ops/edge/ingress.yaml" with { type: "text" };
import nginx from "../ops/edge/resources/nginx.conf" with { type: "text" };
import postinst from "../packaging/deb/postinst" with { type: "text" };
import service from "../packaging/deb/santi.service" with { type: "text" };

assertWebhookBoundary({ ingress, nginx, collection: "/api/v1/webhooks" });
assertDebLinger({ service, postinst, user: "santi", unit: "santi.service" });

await guard(
  [
    { label: "cargo fmt", runs: [["cargo", ["fmt", "--all", "--check"]]] },
    {
      label: "cargo clippy",
      runs: [["cargo", [
        "clippy",
        "--locked",
        "--workspace",
        "--all-targets",
        "--",
        "-D",
        "warnings",
      ]]],
    },
    { label: "cargo test", runs: [["cargo", ["test", "--locked", "--workspace"]]] },
    {
      label: "recovery capsule",
      runs: [["bash", [".runseal/ops/host/30-santi-recovery.test.sh"]]],
    },
    {
      label: "deploy transaction",
      runs: [["bash", [".runseal/ops/host/40-santi-deploy.test.sh"]]],
    },
    { label: "plumb doctor", runs: [["plumb", ["doctor", "."]]] },
    {
      label: "deno fmt",
      runs: [["deno", ["fmt", "--config", ".runseal/deno.json", "--check", ".runseal"]]],
    },
    {
      label: "deno lint",
      runs: [["deno", ["lint", "--config", ".runseal/deno.json", ".runseal"]]],
    },
    {
      label: "deno check",
      runs: [["deno", [
        "check",
        "--config",
        ".runseal/deno.json",
        "--lock",
        ".runseal/deno.lock",
        "--frozen=true",
        ".runseal/wrappers/audit.ts",
        ".runseal/wrappers/deploy.ts",
        ".runseal/wrappers/dev.ts",
        ".runseal/wrappers/guard.ts",
        ".runseal/wrappers/init.ts",
        ".runseal/wrappers/land.ts",
        ".runseal/wrappers/release-ci.ts",
        ".runseal/wrappers/release.ts",
        ".runseal/wrappers/rollback.ts",
        ".runseal/wrappers/santi.ts",
        ".runseal/wrappers/scp.ts",
        ".runseal/wrappers/ssh.ts",
      ]]],
    },
  ],
  Deno.args,
);
