// 手机网页的 Service Worker，只做一件事：安卓分享菜单（需求-0.3 第 11 节）。
// 网页「安装」到安卓主屏幕后，别的 App 分享时选「叮咪」，Chrome 把文字、链接、图片、文件 POST 到 /m/share（以前的网址是 /m/<令牌>/share）。
// 内容要在手机上加密后才发，服务器不能先看到：这里接住，放进缓存，转到 /m/?shared=1（或 /m/<令牌>?shared=1），由网页读出来给用户看一眼再发。
// 别的请求一律不管，照常走网络（不做离线缓存）

const BOX = "dingmi-share";

self.addEventListener("install", () => self.skipWaiting());
self.addEventListener("activate", (e) => e.waitUntil(self.clients.claim()));

self.addEventListener("fetch", (e) => {
  const u = new URL(e.request.url);
  if (e.request.method !== "POST" || !/^\/m\/(?:[^/]+\/)?share$/.test(u.pathname)) return;
  e.respondWith(
    (async () => {
      const back = u.pathname === "/m/share" ? "/m/" : u.pathname.replace(/\/share$/, "");
      try {
        const form = await e.request.formData();
        const text = (k) => String(form.get(k) || "");
        const files = form.getAll("files").filter((f) => typeof f === "object" && f.size > 0);
        await caches.delete(BOX);
        const box = await caches.open(BOX);
        await box.put("/m/share/data", new Response(JSON.stringify({ title: text("title"), text: text("text"), url: text("url"), files: files.map((f) => ({ name: f.name, type: f.type })) })));
        await Promise.all(files.map((f, i) => box.put(`/m/share/file/${i}`, new Response(f, { headers: { "content-type": f.type || "application/octet-stream" } }))));
        return Response.redirect(back + "?shared=1", 303);
      } catch {
        return Response.redirect(back, 303);
      }
    })(),
  );
});
