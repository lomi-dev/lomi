(async () => {
  const invoke = window.__TAURI_INTERNALS__.invoke;
  const wait = async (predicate) => {
    for (let attempt = 0; attempt < 200; attempt++) {
      if (predicate()) return;
      await new Promise((resolve) => setTimeout(resolve, 50));
    }
    throw new Error("Timed out waiting for native application startup.");
  };
  try {
    const entry = SMOKE_ENTRY;
    await wait(() => !!globalThis[Symbol.for("lomi.plugin-api.v1")]);
    let blocked = false;
    try {
      await invoke("prepare_plugin", {
        id: entry.id,
        expected: entry.revision,
      });
    } catch {
      blocked = true;
    }
    if (!blocked)
      throw new Error("Import granted execution trust without approval.");
    await invoke("plugin_smoke_result", { stage: "untrusted", data: entry });
  } catch (error) {
    await invoke("plugin_smoke_result", {
      stage: "failed",
      data: String(error) + "\n" + (error.stack ?? ""),
    });
  }
})();
