(async () => {
  const invoke = window.__TAURI_INTERNALS__.invoke;
  const wait = async (condition) => {
    const deadline = Date.now() + 30000;
    while (Date.now() < deadline) {
      if (await condition()) return;
      await new Promise((resolve) => setTimeout(resolve, 50));
    }
    throw Error(
      "Native agent assertion timed out: " +
        document.body.textContent.slice(-2000),
    );
  };
  let phase = "initial terminal";
  try {
    await wait(() => document.querySelector(".xterm"));
    phase = "background CLI discovery";
    const { cachedInstalledAgentClis } =
      await import("/src/installed-agent-clis.ts");
    const info = await invoke("app_info");
    const session = await invoke("load_session");
    const project = session.projects[0];
    const profile = info.profiles.find((profile) => profile.id === "local:zsh");
    if (!profile) throw Error("Expected the native zsh fixture.");
    await wait(() => cachedInstalledAgentClis(profile, project.path));
    phase = "installed-only dialog";
    document.querySelector('[aria-label^="New tab"]').click();
    await wait(() =>
      document.querySelector('[role="menuitem"][aria-label="Agents"]'),
    );
    const openedAt = performance.now();
    document.querySelector('[role="menuitem"][aria-label="Agents"]').click();
    await wait(() =>
      document.querySelector('.agents-dialog input[type="radio"]'),
    );
    const initialListMs = performance.now() - openedAt;
    if (initialListMs > 500)
      throw Error(`Warmed CLI list took ${initialListMs.toFixed(0)} ms.`);
    const radios = [
      ...document.querySelectorAll('.agents-dialog input[type="radio"]'),
    ];
    if (radios.length !== 1 || radios[0].value !== "cursor")
      throw Error("Expected only the installed Cursor fixture.");
    if (
      document.querySelector('.agents-dialog input[type="number"]').value !==
      "4"
    )
      throw Error("Expected default count 4.");
    phase = "instant cached reopen";
    document
      .querySelector('.agents-dialog button[type="button"].button')
      .click();
    await wait(() => !document.querySelector(".agents-dialog"));
    document.querySelector('[aria-label^="New tab"]').click();
    await wait(() =>
      document.querySelector('[role="menuitem"][aria-label="Agents"]'),
    );
    const reopenedAt = performance.now();
    document.querySelector('[role="menuitem"][aria-label="Agents"]').click();
    await wait(() =>
      document.querySelector('.agents-dialog input[type="radio"]'),
    );
    const reopenedListMs = performance.now() - reopenedAt;
    if (reopenedListMs > 500)
      throw Error(`Reopened CLI list took ${reopenedListMs.toFixed(0)} ms.`);
    phase = "four immediate native PTYs in one split tab";
    document.querySelector('.agents-dialog button[type="submit"]').click();
    await wait(async () => {
      const state = await invoke("plugin_smoke_result", {
        stage: "inspect",
        data: null,
      });
      return state.streamed.trim().split("\n").length === 4;
    });
    await wait(() => !document.querySelector(".agents-dialog"));
    const before = await invoke("plugin_smoke_result", {
      stage: "inspect",
      data: null,
    });
    const launches = before.launches.trim().split("\n");
    if (
      launches.length !== 4 ||
      new Set(launches.map((line) => line.split("\t")[0])).size !== 4
    )
      throw Error("Expected four unique fixture processes.");
    if (launches.some((line) => !line.endsWith("/project")))
      throw Error("Agent cwd changed.");
    const { runningTerminal } = await import("/src/terminal-runtime.ts");
    const { panes } = await import("/src/model.ts");
    let agentTabs;
    await wait(async () => {
      const saved = await invoke("load_session");
      agentTabs = saved.projects[0].workspaces[0].tabs.filter((tab) =>
        tab.title.startsWith("Cursor CLI"),
      );
      return agentTabs.length === 1 && panes(agentTabs[0].layout).length === 4;
    });
    const agentPanes = panes(agentTabs[0].layout);
    const runtimes = agentPanes.map((pane) => runningTerminal(pane.id));
    await wait(() => runtimes.every((runtime) => runtime.opened));
    await wait(() =>
      runtimes.every(
        (runtime) =>
          runtime.getSnapshot().title ===
          `Agent fixture ${before.terminals[runtime.sessionId]}`,
      ),
    );
    phase = "native tab switch continuity";
    const tabs = [...document.querySelectorAll('[role="tab"]')].filter((tab) =>
      tab.textContent.includes("Cursor CLI"),
    );
    if (tabs.length !== 1) throw Error("Expected one Cursor tab.");
    const panels = [...document.querySelectorAll(".terminal-pane")];
    if (panels.length !== 4)
      throw Error("Expected four visible terminal panels.");
    const bounds = agentPanes.map((pane) =>
      document
        .querySelector(`[data-pane-id="${pane.id}"]`)
        .getBoundingClientRect(),
    );
    if (
      Math.abs(bounds[0].y - bounds[1].y) > 1 ||
      Math.abs(bounds[2].y - bounds[3].y) > 1 ||
      Math.abs(bounds[0].x - bounds[2].x) > 1 ||
      Math.abs(bounds[1].x - bounds[3].x) > 1 ||
      bounds[1].x <= bounds[0].x ||
      bounds[2].y <= bounds[0].y
    )
      throw Error("Expected a two-by-two terminal grid.");
    const originalTab = [...document.querySelectorAll('[role="tab"]')].find(
      (tab) => tab.textContent.trim() === "Terminal",
    );
    originalTab.click();
    await wait(() => document.querySelectorAll(".terminal-pane").length === 1);
    tabs[0].click();
    await wait(() => document.querySelectorAll(".terminal-pane").length === 4);
    const titles = new Set();
    for (const [index, pane] of agentPanes.entries()) {
      const panel = document.querySelector(`[data-pane-id="${pane.id}"]`);
      panel.querySelector(".xterm-helper-textarea").focus();
      const expectedTitle = `Agent fixture ${before.terminals[runtimes[index].sessionId]}`;
      await wait(
        () =>
          panel.classList.contains("is-active") &&
          panel.querySelector(".terminal-title")?.textContent === expectedTitle,
      );
      titles.add(panel.querySelector(".terminal-title").textContent);
    }
    if (titles.size !== 4)
      throw Error("Expected four distinct retained titles.");
    const after = await invoke("plugin_smoke_result", {
      stage: "inspect",
      data: null,
    });
    if (
      JSON.stringify(after.terminals) !== JSON.stringify(before.terminals) ||
      after.launches !== before.launches
    )
      throw Error("Switching tabs restarted an agent.");
    phase = "settings main-host compatibility";
    await invoke("plugin_smoke_result", {
      stage: "check-settings",
      data: {
        checks: [
          "installed-only native discovery",
          "background discovery and instant cached dialog reopen",
          "four immediate PTYs in project cwd within one tab",
          "two-by-two terminal grid",
          "output flow control and individual pane titles",
          "tab switches retain processes",
        ],
        launches: 4,
        initialListMs,
        reopenedListMs,
      },
    });
  } catch (error) {
    await invoke("plugin_smoke_result", {
      stage: "failed",
      data: { phase, error: String(error) },
    });
  }
})();
