(async () => {
  const invoke = window.__TAURI_INTERNALS__.invoke;
  const directory = SMOKE_DIRECTORY;
  const bundledThemeSource = SMOKE_THEME_SOURCE;
  try {
    await invoke("open_settings", { page: "themes" });
    const { parseThemeText } = await import("/src/theme/format.ts");
    const { exportVSCodeThemes } = await import("/src/theme/vscode-export.ts");
    const ids = await invoke("import_vscode_themes", {
      path: directory + "/fixture",
    });
    const bundle = await invoke("load_theme", { id: ids[0] });
    const manifest = parseThemeText(bundle.raw);
    const exported = await exportVSCodeThemes(manifest, bundle);
    if (
      JSON.stringify(exported[0].theme.colors) !==
      JSON.stringify(manifest.vscode.colors)
    )
      throw Error("Export changed unedited imported colors");
    const vsix = await invoke("export_vscode_theme", {
      directory,
      name: manifest.name,
      themes: exported,
    });
    const returned = await invoke("import_vscode_themes", { path: vsix });
    const roundtrip = parseThemeText(
      (await invoke("load_theme", { id: returned[0] })).raw,
    );
    if (
      JSON.stringify(roundtrip.vscode.tokenColors) !==
      JSON.stringify(manifest.vscode.tokenColors)
    )
      throw Error("Round trip lost token rules");
    const builtinIds = await invoke("import_vscode_themes", {
      path: directory + "/vscode-defaults",
    });
    if (!builtinIds.length) throw Error("No VS Code theme fixtures imported");
    for (const id of builtinIds)
      parseThemeText((await invoke("load_theme", { id })).raw);
    const iconVsix = [];
    for (const [index, kind] of [
      [1, "file"],
      [2, "product"],
    ]) {
      const icons = await invoke("load_theme", { id: ids[index] });
      if (
        !icons.iconTheme ||
        parseThemeText(icons.raw).iconTheme?.kind !== kind
      )
        throw Error("Missing imported icon theme");
      for (const id of [ids[index], null]) {
        const path = await invoke("export_vscode_icon_theme", {
          directory,
          id,
          kind,
        });
        iconVsix.push(path);
        const imported = await invoke("import_vscode_themes", { path });
        const restored = await invoke("load_theme", { id: imported[0] });
        if (
          Object.keys(restored.iconTheme.iconDefinitions).length !==
          Object.keys(icons.iconTheme.iconDefinitions).length
        )
          throw Error("Icon round trip lost definitions");
      }
    }
    await invoke("save_theme_preferences", {
      data: {
        version: 1,
        active: ids[0],
        appearance: "dark",
        fileIcons: ids[1],
        productIcons: ids[2],
      },
    });
    await invoke("plugin_smoke_result", {
      stage: "theme-exported",
      data: {
        directory,
        vsix,
        iconVsix,
        bundledThemes: builtinIds.length,
        bundledThemeSource,
      },
    });
  } catch (error) {
    await invoke("plugin_smoke_result", {
      stage: "failed",
      data: String(error) + "\n" + (error.stack ?? ""),
    });
  }
})();
