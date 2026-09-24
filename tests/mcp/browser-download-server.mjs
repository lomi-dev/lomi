export function downloadFixture(blockedOrigin) {
  const counts = {};
  return {
    counts,
    serve(request, response) {
      const path = new URL(request.url, "http://fixture").pathname;
      counts[path] = (counts[path] ?? 0) + 1;
      if (path === "/download/binary") {
        if (!request.headers.cookie?.includes("download_session=fixture")) {
          response.writeHead(403).end();
        } else response.end(Buffer.from([0, 255, 65, 13, 10]));
      } else if (path === "/download/empty") {
        response.writeHead(204).end();
      } else if (path === "/download/large") {
        response.end(Buffer.alloc(4 * 1024 * 1024, 165));
      } else if (path === "/download/overflow") {
        response.writeHead(200);
        response.write(Buffer.alloc(8192));
        response.end(Buffer.alloc(8192));
      } else if (path === "/download/redirect") {
        response
          .writeHead(302, { location: `${blockedOrigin}/denied-download` })
          .end();
      } else if (path === "/download/stall") {
        response.writeHead(200);
        response.write("pending");
      } else if (path === "/download/cancel") {
        response.writeHead(200);
        response.write("pending");
        const timer = setTimeout(() => response.end("completed"), 1200);
        response.on("close", () => clearTimeout(timer));
      } else if (path === "/download/counted") {
        response.end(`count:${counts[path]}`);
      } else {
        response.setHeader("Content-Type", "text/html");
        response.setHeader(
          "Set-Cookie",
          "download_session=fixture; HttpOnly; SameSite=Strict; Path=/",
        );
        response.end(
          "<!doctype html><title>Download fixture</title><p>Private browser download fixture</p>",
        );
      }
    },
  };
}
