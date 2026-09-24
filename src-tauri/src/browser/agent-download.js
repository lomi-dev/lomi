// Fixed private-world same-origin GET. No page callback or native IPC.
const q = JSON.parse(payload);
let started = false;
const fail = (error) => JSON.stringify({ error, noEffect: !started });
const current = () => location.origin === q.origin && location.href === q.url;
if (Date.now() >= q.deadlineEpochMs) return fail("DEADLINE_EXCEEDED");
if (!current()) return fail("STALE_SNAPSHOT");
const target = new URL(q.downloadUrl);
if (
  !["http:", "https:"].includes(target.protocol) ||
  target.origin !== q.origin ||
  target.username ||
  target.password ||
  target.hash
)
  return fail("SCOPE_DENIED");
if (!Number.isSafeInteger(q.maxBytes) || q.maxBytes < 1 || q.maxBytes > 4194304)
  return fail("RESOURCE_EXHAUSTED");
const controller = new AbortController();
const timer = setTimeout(
  () => controller.abort(),
  Math.max(0, q.deadlineEpochMs - Date.now()),
);
let reader;
try {
  started = true;
  const response = await fetch(target.href, {
    method: "GET",
    mode: "same-origin",
    credentials: "same-origin",
    redirect: "error",
    cache: "no-store",
    referrerPolicy: "no-referrer",
    signal: controller.signal,
  });
  if (!current()) return fail("STALE_SNAPSHOT");
  if (
    ![200, 204].includes(response.status) ||
    response.redirected ||
    response.url !== target.href
  )
    return fail("UNSUPPORTED_CAPABILITY");
  const declared = response.headers.get("content-length");
  if (declared && /^\d+$/.test(declared) && Number(declared) > q.maxBytes)
    return fail("ARTIFACT_TOO_LARGE");
  const chunks = [];
  let length = 0;
  if (response.body) {
    reader = response.body.getReader();
    for (;;) {
      const { value, done } = await reader.read();
      if (!current()) return fail("STALE_SNAPSHOT");
      if (Date.now() >= q.deadlineEpochMs) return fail("DEADLINE_EXCEEDED");
      if (done) break;
      if (value.byteLength > q.maxBytes - length)
        return fail("ARTIFACT_TOO_LARGE");
      length += value.byteLength;
      // Bound retained objects as well as bytes when a server drips tiny chunks.
      chunks.push(value);
      if (chunks.length > 4096) return fail("RESOURCE_EXHAUSTED");
    }
  }
  const binary = [];
  for (const chunk of chunks) {
    for (let i = 0; i < chunk.length; i += 8192)
      binary.push(String.fromCharCode(...chunk.subarray(i, i + 8192)));
  }
  if (!current()) return fail("STALE_SNAPSHOT");
  return JSON.stringify({ byteLength: length, base64: btoa(binary.join("")) });
} catch {
  return fail(
    controller.signal.aborted ? "DEADLINE_EXCEEDED" : "UNSUPPORTED_CAPABILITY",
  );
} finally {
  clearTimeout(timer);
  controller.abort();
  if (reader) void reader.cancel().catch(() => {});
}
