// Controlled side effect remains in flight until the real helper disconnects.
import { writeFileSync } from "node:fs";
import { join } from "node:path";
export function disconnectFixture(directory) {
  let effects = 0;
  return (request, response) => {
    if (request.url === "/worker.js") {
      response.setHeader("Content-Type", "application/javascript");
      response.end("self.addEventListener('fetch',()=>{});");
      return;
    }
    if (request.method === "POST" && request.url === "/effect") {
      effects++;
      writeFileSync(
        join(directory, "browser-disconnect-effect.json"),
        JSON.stringify({ effects, startedAt: Date.now() }),
      );
      setTimeout(() => response.end("effect:" + effects), 4000);
      return;
    }
    response.setHeader("Content-Type", "text/html; charset=utf-8");
    response.end(`<!doctype html><title>Disconnect fixture</title><button id="submit">Record once</button><p id="result">Ready</p><script>
      document.querySelector('#submit').onclick=()=>{const request=new XMLHttpRequest();request.open('POST','/effect',false);request.send();document.querySelector('#result').textContent=request.responseText;};
    </script>`);
  };
}
