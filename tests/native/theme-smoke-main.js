(async () => {
  const invoke = window.__TAURI_INTERNALS__.invoke;
  let phase = "entry";
  const checkpoint = async (next, details = {}) => {
    phase = next;
    await invoke("plugin_smoke_result", {
      stage: "theme-checkpoint",
      data: { phase, at: new Date().toISOString(), ...details },
    });
  };
  const wait = async (fn) => {
    for (let i = 0; i < 600; i++) {
      if (await fn()) return;
      await new Promise((r) => setTimeout(r, 50));
    }
    throw Error(
      `Native theme smoke timed out at ${phase}: ` +
        document.body.textContent.slice(-1200),
    );
  };
  try {
    await checkpoint("entry", { visibility: document.visibilityState });
    const tab = (name) =>
      [...document.querySelectorAll('[role="tab"]')].find((item) =>
        item.textContent.includes(name),
      );
    await wait(() => tab("source.ts"));
    const startup = await invoke("agent_control_startup_state");
    if (startup.supported && startup.autoStart === null) {
      await wait(() =>
        [...document.querySelectorAll("dialog[open] button")].some(
          (button) => button.textContent.trim() === "Keep disabled",
        ),
      );
      [...document.querySelectorAll("dialog[open] button")]
        .find((button) => button.textContent.trim() === "Keep disabled")
        .click();
      await wait(() => !document.querySelector("dialog[open]"));
    }
    await checkpoint("startup-consent-settled");
    tab("source.ts").click();
    await wait(() => document.querySelector(".file-editor .cm-content"));
    await checkpoint("editor-ready");
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
    tab("Terminal").click();
    const { runningTerminal } = await import("/src/terminal-runtime.ts");
    let terminal;
    await wait(() => {
      terminal = runningTerminal("theme-terminal-pane");
      return terminal?.getSnapshot().status === "running";
    });
    const id = terminal.sessionId;
    await checkpoint("terminal-ready", {
      sessionId: id,
      status: terminal.getSnapshot().status,
    });
    const inspectSettings = () =>
      invoke("plugin_smoke_result", { stage: "theme-inspect", data: null });
    const nativeBefore = await inspectSettings();
    const settingsTab = () =>
      document.querySelector('[data-tab-id="settings"] [role="tab"]');
    const pageSelected = (page) =>
      document
        .querySelector('.settings-nav-item[aria-current="page"]')
        ?.textContent.trim() === page &&
      settingsTab()?.getAttribute("aria-selected") === "true";
    for (const [page, label] of [
      ["keybinds", "Keybinds"],
      ["terminal", "Terminal"],
      ["themes", "Themes"],
      ["about", "About"],
    ]) {
      await checkpoint("settings-page", { page });
      await invoke("open_settings", { page });
      await wait(() => pageSelected(label));
      await invoke("open_settings", { page });
      await wait(() => pageSelected(label));
      if (document.querySelectorAll('[data-tab-id="settings"]').length !== 1)
        throw Error("Repeated open duplicated the Settings tab");
      const native = await inspectSettings();
      if (
        native.windows.length !== 1 ||
        native.windows[0] !== "main" ||
        !native.mainVisible
      )
        throw Error("Opening Settings changed the native window set");
    }
    document.querySelector('[aria-label="Close Settings"]').click();
    await wait(() => !settingsTab());
    await checkpoint("settings-closed");
    const nativeAfter = await inspectSettings();
    if (
      nativeAfter.windows.length !== 1 ||
      nativeAfter.windows[0] !== "main" ||
      !nativeAfter.mainVisible
    )
      throw Error("Closing Settings closed or hid the main window");
    if (
      JSON.stringify(nativeBefore.terminals) !==
        JSON.stringify(nativeAfter.terminals) ||
      runningTerminal("theme-terminal-pane") !== terminal ||
      terminal.sessionId !== id
    )
      throw Error(
        "Settings navigation replaced a PTY: " +
          JSON.stringify({
            before: nativeBefore.terminals,
            after: nativeAfter.terminals,
            sameRuntime: runningTerminal("theme-terminal-pane") === terminal,
            sessionId: terminal.sessionId,
            expectedSessionId: id,
          }),
      );
    if (
      documents().find((entry) => entry === buffer) !== buffer ||
      buffer.state.doc.toString() !== "const value = 99;\n"
    )
      throw Error("Settings navigation replaced the dirty editor buffer");
    await invoke("open_settings");
    await wait(() => pageSelected("About"));
    document.querySelector('[aria-label="Close Settings"]').click();
    await wait(() => !settingsTab());
    await checkpoint("settings-retention-verified");
    await invoke("plugin_smoke_result", {
      stage: "theme-main-ready",
      data: null,
    });
    await wait(() => window.__themeSmokeExport);
    const currentTheme = await invoke("load_theme_preferences");
    const themeCatalog = await invoke("list_themes");
    const { effectiveTheme } = await import("/src/theme/runtime.ts");
    await checkpoint("theme-export-diagnostics", {
      terminalBlue: terminal.terminal.options.theme?.blue,
      computedTerminalBlue: getComputedStyle(document.documentElement)
        .getPropertyValue("--terminal-blue")
        .trim(),
      nativePreferences: currentTheme.preferences,
      nativeRevision: currentTheme.revision,
      nativeThemeId: currentTheme.theme?.id,
      nativeThemeError: currentTheme.theme?.error,
      runtimeRevision: effectiveTheme().sourceRevision,
      runtimeBlue: effectiveTheme().terminal.theme?.blue,
      domTheme: document.documentElement.dataset.theme,
      domRevision: document.documentElement.dataset.themeSourceRevision,
      catalogCount: themeCatalog.themes.length,
      themeLayers: [...document.querySelectorAll("[data-theme-layer]")].map(
        (node) => ({
          layer: node.dataset.themeLayer,
          builtin: node.dataset.themeBuiltin,
          media: node.media,
        }),
      ),
      openDialogs: [...document.querySelectorAll("dialog[open]")].map((node) =>
        node.textContent.slice(0, 1000),
      ),
      notices: [...document.querySelectorAll(".settings-notice")].map((node) =>
        node.textContent.slice(0, 1000),
      ),
    });
    await wait(() => terminal.terminal.options.theme.blue === "#345678ff");
    await checkpoint("themes-imported");
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
    await checkpoint("theme-retention-verified");
    await invoke("plugin_smoke_result", {
      stage: "passed",
      data: {
        checks: [
          "one main native window while Settings opens and closes",
          "repeated Settings opens reuse one tab and deep links navigate pages",
          "no-page reopen retains selected Settings page",
          "Settings close retains PTY identity and dirty editor buffer",
          "native VS Code import",
          "file SVG and product font icons rendered in WKWebView",
          "imported and builtin icon VSIX exported and reimported with assets",
          "independent icon preferences",
          "native WKWebView Settings-tab export",
          window.__themeSmokeExport.bundledThemeSource === "owned-fixture"
            ? "owned VS Code compatible theme fixtures parsed"
            : "real VS Code bundled themes parsed",
          "native Settings-tab theme updates",
          "PTY identity and output retained",
          "dirty editor text and undo retained",
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
