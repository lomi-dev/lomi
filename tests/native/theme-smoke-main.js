(async () => {
  const invoke = window.__TAURI_INTERNALS__.invoke;
  const wait = async (fn) => {
    for (let i = 0; i < 600; i++) {
      if (await fn()) return;
      await new Promise((r) => setTimeout(r, 50));
    }
    throw Error(
      "Native theme smoke timed out: " + document.body.textContent.slice(-1200),
    );
  };
  try {
    await wait(() => document.querySelector(".file-editor .cm-content"));
    const { documents } = await import("/src/editor-runtime.ts");
    const buffer = documents()[0];
    await wait(() => buffer.view);
    buffer.view.dispatch({
      changes: {
        from: 0,
        to: buffer.state.doc.length,
        insert: "const value = 99;\n",
      },
    });
    const tab = (name) =>
      [...document.querySelectorAll('[role="tab"]')].find((t) =>
        t.textContent.includes(name),
      );
    tab("Terminal").click();
    const { runningTerminal } = await import("/src/terminal-runtime.ts");
    let terminal;
    await wait(
      () => (terminal = runningTerminal("theme-terminal-pane"))?.sessionId,
    );
    const id = terminal.sessionId;
    await invoke("plugin_smoke_result", {
      stage: "theme-main-ready",
      data: null,
    });
    await wait(
      () =>
        window.__themeSmokeExport &&
        terminal.terminal.options.theme.blue === "#345678ff",
    );
    if (terminal.sessionId !== id)
      throw Error("Theme restarted the native PTY");
    await invoke("write_terminal", {
      id,
      data: "printf 'THEME_NATIVE_OK\\n'\n",
    });
    await wait(() =>
      Array.from({ length: terminal.terminal.buffer.active.length }, (_, i) =>
        terminal.terminal.buffer.active.getLine(i)?.translateToString(),
      )
        .join("\n")
        .includes("THEME_NATIVE_OK"),
    );
    tab("source.ts").click();
    await wait(() => buffer.view);
    if (buffer.state.doc.toString() !== "const value = 99;\n")
      throw Error("Theme lost dirty editor text");
    buffer.command("undo");
    if (buffer.state.doc.toString() !== "const value = 1;\n")
      throw Error("Theme lost undo history");
    await wait(
      () =>
        document.querySelector('.editor-heading [data-file-icon="file"]') &&
        document.querySelector('[data-product-icon="settings-gear"] text'),
    );
    const glyph = document.querySelector(
      '[data-product-icon="settings-gear"] text',
    );
    const family = getComputedStyle(glyph).fontFamily;
    if (
      !family.includes("Lomi icons") ||
      !document.fonts.check(`16px ${family}`, glyph.textContent) ||
      !glyph.getBBox().width
    )
      throw Error("Native product icon font did not render");
    const iconPreferences = await invoke("load_theme_preferences");
    if (
      !iconPreferences.fileIcons?.iconTheme ||
      !iconPreferences.productIcons?.iconTheme
    )
      throw Error("Native icon preferences missing");
    let iconDenied = false;
    try {
      await invoke("export_vscode_icon_theme", {
        directory: window.__themeSmokeExport.directory,
        id: null,
        kind: "file",
      });
    } catch (error) {
      iconDenied = String(error).includes("not available in this window");
    }
    if (!iconDenied) throw Error("Main window could export icon themes");
    let denied = false;
    try {
      await invoke("export_vscode_theme", {
        directory: window.__themeSmokeExport.directory,
        name: "Denied",
        themes: [{ name: "Denied", theme: { type: "dark", colors: {} } }],
      });
    } catch (error) {
      denied = String(error).includes("not available in this window");
    }
    if (!denied) throw Error("Main window could export themes");
    await invoke("plugin_smoke_result", {
      stage: "passed",
      data: {
        checks: [
          "native VS Code import",
          "file SVG and product font icons rendered in WKWebView",
          "imported and builtin icon VSIX exported and reimported with assets",
          "independent icon preferences and main-window export denial",
          "native WKWebView isolated export",
          "real VS Code bundled themes parsed",
          "native cross-window theme updates",
          "PTY identity and output retained",
          "dirty editor text and undo retained",
          "main-window export denied",
        ],
        ...window.__themeSmokeExport,
      },
    });
  } catch (error) {
    await invoke("plugin_smoke_result", {
      stage: "failed",
      data: String(error) + "\n" + (error.stack ?? ""),
    });
  }
})();
