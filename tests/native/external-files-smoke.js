(async () => {
  const { root, files } = SMOKE_CONFIG;
  const invoke = window.__TAURI_INTERNALS__?.invoke;
  const checks = [];
  const assert = (value, message) => {
    if (!value) throw Error(message);
  };
  const wait = async (condition, message, timeout = 30_000) => {
    const deadline = Date.now() + timeout;
    while (Date.now() < deadline) {
      if (await condition()) return;
      await new Promise((resolve) => setTimeout(resolve, 50));
    }
    throw Error(`Timed out: ${message}`);
  };
  const tabs = () => [...document.querySelectorAll('[role="tab"]')];
  const findTab = (name) =>
    tabs().find(
      (tab) =>
        tab.querySelector(":scope > span:not(.tab-modified)")?.textContent ===
          name || tab.getAttribute("aria-label") === name,
    );
  const editorText = () =>
    [...document.querySelectorAll(".file-editor .cm-content .cm-line")]
      .map((line) => line.textContent ?? "")
      .join("\n");
  const clickTab = async (name) => {
    await wait(() => findTab(name), `tab ${name}`);
    findTab(name).click();
  };
  const waitForAcknowledged = async (paths) =>
    wait(async () => {
      const pending = await invoke("external_files_pending");
      return !pending.some((request) => paths.includes(request.path));
    }, "external file requests acknowledged");
  const assertTextEditor = async (name, text) => {
    await clickTab(name);
    await wait(() => editorText() === text, `CodeMirror contents for ${name}`);
  };
  const expectReject = async (command, args, message, detail) => {
    let error = "";
    try {
      await invoke(command, args);
    } catch (reason) {
      error = String(reason?.message ?? reason);
    }
    assert(error, `${detail} was accepted`);
    if (message)
      assert(
        error.includes(message),
        `${detail} returned an unexpected error: ${error}`,
      );
  };

  try {
    assert(typeof invoke === "function", "Tauri invoke is unavailable");
    await invoke("plugin_smoke_result", {
      stage: "script-started",
      data: { href: window.location.href },
    });
    const coldNames = [
      files.spaced,
      files.unknown,
      files.dotenv,
      files.extensionless,
      files.image,
      files.second.split("/").at(-1),
    ];
    await wait(
      () => coldNames.every((name) => findTab(name)),
      "cold-start OS files in workspace tabs",
    );
    await waitForAcknowledged(
      coldNames.map(
        (name) => `${root}/${name === "second file.txt" ? files.second : name}`,
      ),
    );
    assert(
      coldNames.every(
        (name) =>
          tabs().filter((tab) => tab.textContent?.includes(name)).length === 1,
      ),
      "Cold-start open did not create exactly one tab per file",
    );
    checks.push(
      "cold Launch Services open restores multiple files as unique tabs",
    );

    await assertTextEditor(files.spaced, "Dirty original — żółć\n");
    await assertTextEditor(files.unknown, "Unknown extension opened as text\n");
    await assertTextEditor(files.dotenv, "TOKEN=smoke\n");
    await assertTextEditor(files.extensionless, "Extensionless text opened\n");
    await assertTextEditor("second file.txt", "Second OS-opened file\n");
    checks.push(
      "Unicode and spaced names, unknown extensions, .env, extensionless text and nested paths render in CodeMirror",
    );

    await clickTab(files.image);
    await wait(() => {
      const image = document.querySelector(".image-canvas img");
      return image?.complete && image.naturalWidth > 0;
    }, "native image preview");
    assert(
      !document.querySelector(".file-editor .cm-content"),
      "Image opened as a text editor",
    );
    checks.push("OS-opened PNG renders in the native image preview");

    await clickTab(files.spaced);
    const editor = document.querySelector(".file-editor .cm-content");
    assert(editor, "Dirty-file editor is missing");
    editor.focus();
    assert(document.execCommand("selectAll"), "CodeMirror select-all failed");
    assert(
      document.execCommand(
        "insertText",
        false,
        "Unsaved OS-open draft — żółć 😀",
      ),
      "CodeMirror insertText failed",
    );
    await wait(
      () => editorText() === "Unsaved OS-open draft — żółć 😀",
      "dirty editor buffer",
    );
    checks.push("dirty editor buffer prepared before warm OS open");
    await invoke("hide_main_window");
    await wait(
      async () =>
        !(await invoke("plugin_smoke_result", {
          stage: "window-visible",
          data: null,
        })),
      "main window hidden before warm OS open",
    );
    checks.push("main window hidden before warm OS open");
    await invoke("plugin_smoke_result", {
      stage: "cold-ready",
      data: { checks },
    });

    await wait(
      () => findTab(files.warm) && findTab(files.binary),
      "warm OS-open files",
      120_000,
    );
    await waitForAcknowledged([
      `${root}/${files.dirty}`,
      `${root}/${files.warm}`,
      `${root}/${files.binary}`,
    ]);
    await wait(
      async () =>
        await invoke("plugin_smoke_result", {
          stage: "window-visible",
          data: null,
        }),
      "warm OS open presents the hidden main window",
    );
    checks.push(
      "warm OS open presents the hidden main window in the existing process",
    );

    assert(
      tabs().filter((tab) => tab.textContent?.includes(files.spaced)).length ===
        1,
      "Warm opening duplicated the existing dirty file tab",
    );
    await assertTextEditor(files.spaced, "Unsaved OS-open draft — żółć 😀");
    assert(
      document
        .querySelector(".file-editor .editor-path")
        ?.textContent?.includes("•"),
      "Warm opening cleared the dirty editor state",
    );
    await assertTextEditor(files.warm, "Warm OS-open file\n");
    checks.push(
      "warm Launch Services open reuses the dirty tab and preserves unsaved Unicode text",
    );

    await clickTab(files.binary);
    await wait(
      () =>
        document
          .querySelector(".editor-loading")
          ?.textContent?.includes("not supported text"),
      "binary editor rejection",
    );
    assert(
      !document.querySelector(".file-editor .cm-content"),
      "Binary file opened as text",
    );
    checks.push(
      "OS-opened binary file displays the editor's unsupported-text error",
    );

    await expectReject(
      "resolve_external_file",
      { path: `${root}/${files.missing}`, roots: [root] },
      "Cannot open",
      "missing file",
    );
    await expectReject(
      "resolve_external_file",
      { path: `${root}/${files.directory}`, roots: [root] },
      "Only regular files",
      "directory",
    );
    await expectReject(
      "read_editor_file",
      { root, relative: files.binary },
      "not supported",
      "binary text read",
    );
    checks.push(
      "missing paths, directories and binary text reads are rejected",
    );

    await invoke("plugin_smoke_result", {
      stage: "warm-ready",
      data: { checks },
    });
    await invoke("plugin_smoke_result", {
      stage: "passed",
      data: {
        checks,
        tabCount: tabs().length,
        dirtyBuffer: editorText(),
      },
    });
  } catch (error) {
    if (typeof invoke === "function")
      await invoke("plugin_smoke_result", {
        stage: "failed",
        data: {
          error: String(error),
          stack: String(error?.stack ?? ""),
          checks,
        },
      });
    else console.error(error);
  }
})();
