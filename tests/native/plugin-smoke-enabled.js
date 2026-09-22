(async () => {
  const invoke = window.__TAURI_INTERNALS__.invoke;
  const wait = async (fn) => {
    for (let i = 0; i < 200; i++) {
      if (await fn()) return;
      await new Promise((r) => setTimeout(r, 50));
    }
    throw Error(
      "Native assertion timed out: " + document.body.textContent.slice(-3000),
    );
  };
  const button = (text, scope = document) =>
    [...scope.querySelectorAll("button")].find(
      (button) => button.textContent.trim() === text,
    );
  const command = async (label) => {
    window.dispatchEvent(
      new KeyboardEvent("keydown", {
        key: "P",
        code: "KeyP",
        ctrlKey: !navigator.platform.includes("Mac"),
        metaKey: navigator.platform.includes("Mac"),
        shiftKey: true,
        bubbles: true,
      }),
    );
    await wait(() => document.querySelector('[aria-label="Find command"]'));
    const input = document.querySelector('[aria-label="Find command"]');
    Object.getOwnPropertyDescriptor(
      HTMLInputElement.prototype,
      "value",
    ).set.call(input, label);
    input.dispatchEvent(new Event("input", { bubbles: true }));
    await wait(() =>
      [...document.querySelectorAll(".command-picker-list button")].some(
        (b) => b.textContent.startsWith(label) && !b.disabled,
      ),
    );
    [...document.querySelectorAll(".command-picker-list button")]
      .find((b) => b.textContent.startsWith(label))
      .click();
  };
  try {
    const entry = SMOKE_ENTRY;
    await wait(() => document.querySelector(".terminal-pane"));
    await wait(
      async () =>
        Object.keys(
          (
            await invoke("plugin_smoke_result", {
              stage: "inspect",
              data: null,
            })
          ).terminals,
        ).length === 1,
    );
    const before = await invoke("plugin_smoke_result", {
      stage: "inspect",
      data: null,
    });
    if (Object.keys(before.terminals).length !== 1)
      throw Error("Expected one live initial shell");
    const terminal = Object.keys(before.terminals)[0];
    globalThis.__lomiSmoke = { before, terminal, entry };
    await invoke("write_terminal", {
      id: terminal,
      data: "for i in 1 2 3 4 5 6 7 8 9 10; do printf '\\nLOMI_NATIVE_%s\\n' \"$i\"; sleep 0.1; done\n",
    });
    await command("Show workspace context");
    await wait(() =>
      document
        .querySelector(".context-plugin")
        ?.textContent.includes("Native workspace"),
    );
    await wait(
      () => document.querySelector(".context-plugin img")?.naturalWidth > 0,
    );
    if (
      getComputedStyle(document.querySelector(".context-plugin")).overflow !==
      "auto"
    )
      throw Error("Package stylesheet did not apply");
    button("Show details").click();
    await wait(() =>
      document
        .querySelector(".context-plugin")
        ?.textContent.includes("relative ESM chunk"),
    );
    await command("Dock current tab");
    await wait(() => button("Dock"));
    button("Dock").click();
    await wait(() => document.querySelector(".dock-pane-host .context-plugin"));
    const after = await invoke("plugin_smoke_result", {
      stage: "inspect",
      data: null,
    });
    if (JSON.stringify(before.terminals) !== JSON.stringify(after.terminals))
      throw Error("Docking restarted the PTY");
    await wait(() => button("Hide details"));
    await invoke("plugin_smoke_result", { stage: "workbench-ready", data: {} });
  } catch (error) {
    await invoke("plugin_smoke_result", {
      stage: "failed",
      data: String(error) + "\n" + (error.stack ?? ""),
    });
  }
})();
