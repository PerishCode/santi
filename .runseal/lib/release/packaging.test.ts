import postinst from "../../packaging/deb/postinst" with { type: "text" };
import service from "../../packaging/deb/santi.service" with { type: "text" };

function requireMatch(text: string, pattern: RegExp): void {
  if (!pattern.test(text)) {
    throw new Error(`missing packaging contract ${pattern}`);
  }
}

Deno.test("deb keeps the detached job user manager alive across cold boots", () => {
  requireMatch(service, /^\s*User=santi\s*$/m);
  requireMatch(service, /^\s*PAMName=login\s*$/m);
  requireMatch(postinst, /^\s*loginctl enable-linger santi\s*$/m);
  if (
    postinst.indexOf("loginctl enable-linger santi") >=
      postinst.indexOf("systemctl enable santi.service")
  ) {
    throw new Error("lingering must be enabled before the runtime service");
  }
});
