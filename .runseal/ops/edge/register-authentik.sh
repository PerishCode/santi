#!/bin/sh
# Idempotently provision Santi's Authentik forward-auth application and CLI identity.
set -eu

AK_TOKEN="$(k3s kubectl get secret authentik-secret -n authentik \
  -o jsonpath='{.data.AUTHENTIK_BOOTSTRAP_TOKEN}' | base64 -d)"
export AK_TOKEN
export AK_CFG='{"name":"santi-window","external_host":"https://santi.liberte.top","cookie_domain":"liberte.top","auth_base":"https://auth.liberte.top"}'

python3 - <<'PYEOF'
import json
import os
import ssl
import urllib.error
import urllib.request

cfg = json.loads(os.environ["AK_CFG"])
base = cfg["auth_base"] + "/api/v3"
headers = {
    "Authorization": "Bearer " + os.environ["AK_TOKEN"],
    "Content-Type": "application/json",
}
context = ssl.create_default_context()
context.check_hostname = False
context.verify_mode = ssl.CERT_NONE


def api(method, path, body=None):
    request = urllib.request.Request(
        base + path,
        data=json.dumps(body).encode() if body is not None else None,
        headers=headers,
        method=method,
    )
    try:
        with urllib.request.urlopen(request, context=context, timeout=30) as response:
            raw = response.read().decode()
            return response.status, json.loads(raw) if raw else {}
    except urllib.error.HTTPError as error:
        raw = error.read().decode()
        try:
            return error.code, json.loads(raw)
        except json.JSONDecodeError:
            return error.code, raw


def first(response):
    results = response.get("results", [])
    return results[0] if results else None


name = cfg["name"]
_, auth_flows = api("GET", "/flows/instances/?designation=authorization&ordering=slug")
_, invalidation_flows = api("GET", "/flows/instances/?designation=invalidation&ordering=slug")
authorization = next(
    (
        row["pk"]
        for row in auth_flows["results"]
        if row["slug"] == "default-provider-authorization-implicit-consent"
    ),
    auth_flows["results"][0]["pk"],
)
invalidation = next(
    (
        row["pk"]
        for row in invalidation_flows["results"]
        if row["slug"] == "default-provider-invalidation-flow"
    ),
    invalidation_flows["results"][0]["pk"],
)

_, providers = api("GET", "/providers/proxy/?search=" + name)
provider = next(
    (row for row in providers.get("results", []) if row.get("name") == name), None
)
if not provider:
    status, provider = api(
        "POST",
        "/providers/proxy/",
        {
            "name": name,
            "authorization_flow": authorization,
            "invalidation_flow": invalidation,
            "external_host": cfg["external_host"],
            "mode": "forward_domain",
            "cookie_domain": cfg["cookie_domain"],
        },
    )
    assert status < 300, provider
client_id = provider["client_id"]
provider_pk = provider["pk"]

status, application = api("GET", "/core/applications/" + name + "/")
if status >= 300:
    status, application = api(
        "POST",
        "/core/applications/",
        {"name": name, "slug": name, "provider": provider_pk},
    )
    assert status < 300, application

_, outposts = api("GET", "/outposts/instances/?ordering=name")
embedded = next(row for row in outposts["results"] if "Embedded" in row["name"])
patch = {}
provider_ids = embedded.get("providers") or []
if provider_pk not in provider_ids:
    patch["providers"] = provider_ids + [provider_pk]
outpost_config = embedded.get("config") or {}
if outpost_config.get("authentik_host") != cfg["auth_base"]:
    outpost_config["authentik_host"] = cfg["auth_base"]
    patch["config"] = outpost_config
if patch:
    status, result = api("PATCH", "/outposts/instances/" + embedded["pk"] + "/", patch)
    assert status < 300, result

username = name + "-cli"
_, users = api("GET", "/core/users/?username=" + username)
service_account = first(users)
if not service_account:
    status, result = api(
        "POST",
        "/core/users/service_account/",
        {"name": username, "create_group": False, "expiring": False},
    )
    assert status < 300, result
    user_pk = result["user_pk"]
    username = result["username"]
else:
    user_pk = service_account["pk"]
    username = service_account["username"]

token_id = name + "-app-password"
status, _ = api("GET", "/core/tokens/" + token_id + "/")
if status >= 300:
    status, result = api(
        "POST",
        "/core/tokens/",
        {
            "identifier": token_id,
            "intent": "app_password",
            "user": user_pk,
            "expiring": False,
        },
    )
    assert status < 300, result
_, view = api("GET", "/core/tokens/" + token_id + "/view_key/")

print("provider '" + name + "' + application bound to embedded outpost")
print("SANTI_AUTH_TOKEN_URL=" + cfg["auth_base"] + "/application/o/token/")
print("SANTI_AUTH_CLIENT_ID=" + client_id)
print("SANTI_AUTH_USERNAME=" + username)
print("SANTI_AUTH_PASSWORD=" + view["key"])
PYEOF
