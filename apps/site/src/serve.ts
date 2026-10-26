import { resolve, sep } from "node:path";
import { stat } from "node:fs/promises";

const root = resolve(import.meta.dir, "../dist");
const base = process.env.SITE_BASE_PATH || "/";
const server = Bun.serve({
  hostname: "0.0.0.0",
  port: Number(process.env.PORT || 4321),
  async fetch(request) {
    const url = new URL(request.url);
    const pathname = decodeURIComponent(url.pathname);
    if (!pathname.startsWith(base))
      return new Response("Not found", { status: 404 });
    const target = resolve(root, pathname.slice(base.length) || "index.html");
    if (target !== root && !target.startsWith(`${root}${sep}`))
      return new Response("Not found", { status: 404 });
    let file = Bun.file(target);
    if ((await stat(target).catch(() => null))?.isDirectory()) {
      if (!pathname.endsWith("/"))
        return Response.redirect(`${url.origin}${pathname}/${url.search}`, 301);
      file = Bun.file(resolve(target, "index.html"));
    }
    if (await file.exists()) return new Response(file);
    return new Response(Bun.file(resolve(root, "404.html")), { status: 404 });
  },
});
console.log(`Pingo site: http://localhost:${server.port}${base}`);
