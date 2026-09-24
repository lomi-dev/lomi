import { createHash } from "node:crypto";
export function uploadFixture() {
  const received = [];
  return {
    received,
    serve(request, response) {
      const url = new URL(request.url, "http://fixture");
      if (url.pathname === "/uploaded" && request.method === "POST") {
        const hash = createHash("sha256");
        let length = 0;
        request.on("data", (bytes) => {
          length += bytes.length;
          if (length > 4194304) request.destroy();
          else hash.update(bytes);
        });
        request.on("end", () => {
          received.push({ length, sha256: hash.digest("hex") });
          response.end("received");
        });
        return;
      }
      response.setHeader("Content-Type", "text/html");
      response.end(`<!doctype html><title>Upload fixture</title><style>input{display:block;margin:24px}iframe{width:90%;height:260px}</style><label>${url.pathname === "/child" ? "Child" : "Main"} file<input id="file" type="file"></label>${url.pathname === "/child" ? "" : '<iframe src="/child"></iframe>'}<script>
      globalThis.uploads=[];globalThis.events=[];
      for(const type of ['input','change']) file.addEventListener(type,e=>events.push([e.type,e.isTrusted]));
      file.addEventListener('change',async()=>{const f=file.files[0];const bytes=await f.arrayBuffer();const digest=await crypto.subtle.digest('SHA-256',bytes);const hash=Array.from(new Uint8Array(digest),x=>x.toString(16).padStart(2,'0')).join('');await fetch('/uploaded',{method:'POST',body:bytes});uploads.push({name:f.name,byteLength:f.size,sha256:hash});});
    </script>`);
    },
  };
}
