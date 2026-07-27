import ingress from "../../ops/edge/ingress.yaml" with { type: "text" };
import nginx from "../../ops/edge/resources/nginx.conf" with { type: "text" };

Deno.test("webhook bypass excludes the management collection", () => {
  if (!ingress.includes("PathPrefix(`/api/v1/webhooks/`)")) {
    throw new Error("Traefik webhook bypass must require the trailing slash");
  }
  if (ingress.includes("PathPrefix(`/api/v1/webhooks`)")) {
    throw new Error("Traefik webhook bypass exposes the exact management collection");
  }
  if (!nginx.includes("location /api/v1/webhooks/ {")) {
    throw new Error("nginx must exempt only named webhook event paths");
  }
  if (!nginx.includes("location = /api/v1/webhooks {")) {
    throw new Error("nginx must keep the management collection exact to avoid a public redirect");
  }
});
