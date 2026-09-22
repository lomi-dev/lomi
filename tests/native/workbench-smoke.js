(async () => {
  const invoke = window.__TAURI_INTERNALS__.invoke;
  let phase = "theme selection";
  const wait = async (fn) => {
    for (let i = 0; i < 200; i++) {
      if (await fn()) return;
      await new Promise((r) => setTimeout(r, 50));
    }
    throw Error(
      "Native workbench assertion timed out: " +
        document.body.textContent.slice(-2500),
    );
  };
  const button = (text) =>
    [...document.querySelectorAll("button")].find(
      (button) => button.textContent.trim() === text,
    );
  try {
    const { before, terminal } = globalThis.__lomiSmoke;
    phase = "Explorer native directory watch without Git";
    await wait(() => document.querySelector(".file-tree"));
    await invoke("write_terminal", {
      id: terminal,
      data: "mkdir explorer-watch\n",
    });
    await wait(() => button("explorer-watch"));
    button("explorer-watch").click();
    await invoke("write_terminal", {
      id: terminal,
      data: "for i in $(seq 1 100); do touch explorer-watch/file-$i; done; touch explorer-watch/.hidden\n",
    });
    await wait(() => button("file-100") && button(".hidden"));
    await invoke("write_terminal", {
      id: terminal,
      data: "mv explorer-watch/file-100 explorer-watch/renamed; rm explorer-watch/.hidden\n",
    });
    await wait(
      () => button("renamed") && !button("file-100") && !button(".hidden"),
    );
    button("explorer-watch").click();
    await invoke("write_terminal", {
      id: terminal,
      data: "touch explorer-watch/while-collapsed\n",
    });
    button("explorer-watch").click();
    await wait(() => button("while-collapsed"));
    phase = "theme selection";
    await wait(() => document.documentElement.dataset.theme === "theme-copy");
    const canvasCount = document.querySelectorAll(".xterm canvas").length;
    phase = "active theme watch";
    const previousRevision =
      document.documentElement.dataset.themeSourceRevision;
    await invoke("plugin_smoke_result", { stage: "theme-write", data: null });
    await wait(
      () =>
        getComputedStyle(document.documentElement)
          .getPropertyValue("--radius-control")
          .trim() === "11px" &&
        document.documentElement.dataset.appearance === "light" &&
        document.documentElement.dataset.themeSourceRevision !==
          previousRevision,
    );
    phase = "settings watch convergence";
    await invoke("plugin_smoke_result", {
      stage: "check-settings",
      data: document.documentElement.dataset.themeSourceRevision,
    });
    await wait(() => globalThis.__lomiSmokeSettings);
    if (
      globalThis.__lomiSmokeSettings.sourceRevision !==
      document.documentElement.dataset.themeSourceRevision
    )
      throw Error("Windows have different theme source revisions");
    phase = "native browser load";
    document
      .querySelector('[data-tab-id="native-browser"] [role="tab"]')
      .click();
    await wait(async () => {
      const state = await invoke("plugin_smoke_result", {
        stage: "inspect",
        data: null,
      });
      return (
        state.browsers["native-browser"]?.title === "Native browser smoke" &&
        state.browsers["native-browser"].visible
      );
    });
    const browserBefore = (
      await invoke("plugin_smoke_result", { stage: "inspect", data: null })
    ).browsers["native-browser"];
    phase = "browser IPC and input";
    await invoke("plugin_smoke_result", { stage: "browser-probe", data: null });
    await wait(
      async () =>
        (await invoke("plugin_smoke_result", { stage: "inspect", data: null }))
          .browsers["native-browser"].title === "Native browser isolated",
    );
    phase = "native browser panel resize";
    document.querySelector('[aria-label="Resize sidebar"]').dispatchEvent(
      new KeyboardEvent("keydown", {
        key: "ArrowRight",
        code: "ArrowRight",
        bubbles: true,
      }),
    );
    await wait(
      async () =>
        Math.abs(
          (
            await invoke("plugin_smoke_result", {
              stage: "inspect",
              data: null,
            })
          ).browsers["native-browser"].bounds[2] - browserBefore.bounds[2],
        ) > 2,
    );
    phase = "native browser zoom";
    window.dispatchEvent(
      new KeyboardEvent("keydown", {
        key: "=",
        code: "Equal",
        ctrlKey: true,
        bubbles: true,
      }),
    );
    await wait(
      () =>
        getComputedStyle(document.documentElement)
          .getPropertyValue("--app-zoom")
          .trim() === "1.1",
    );
    await wait(async () => {
      const bounds = (
        await invoke("plugin_smoke_result", { stage: "inspect", data: null })
      ).browsers["native-browser"].bounds;
      const rect = document
        .querySelector(".browser-viewport")
        .getBoundingClientRect();
      return (
        Math.abs(bounds[0] - rect.x * 1.1) < 2 &&
        Math.abs(bounds[1] - rect.y * 1.1) < 2 &&
        Math.abs(bounds[2] - rect.width * 1.1) < 2 &&
        Math.abs(bounds[3] - rect.height * 1.1) < 2
      );
    });
    phase = "native browser dialog visibility";
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
    await wait(
      async () =>
        !(await invoke("plugin_smoke_result", { stage: "inspect", data: null }))
          .browsers["native-browser"].visible,
    );
    document.querySelector('[aria-label="Close dialog"]').click();
    await wait(
      async () =>
        (await invoke("plugin_smoke_result", { stage: "inspect", data: null }))
          .browsers["native-browser"].visible,
    );
    await invoke("plugin_smoke_result", {
      stage: "browser-retained",
      data: null,
    });
    await wait(
      async () =>
        (await invoke("plugin_smoke_result", { stage: "inspect", data: null }))
          .browsers["native-browser"].title === "Native browser retained",
    );
    phase = "terminal parsing after theme and browser";
    document
      .querySelector('[data-tab-id="native-terminal"] [role="tab"]')
      .click();
    await wait(() => document.querySelector(".context-plugin"));
    await invoke("write_terminal", {
      id: terminal,
      data: "printf '\\nLOMI_NATIVE_AFTER_THEME\\n'\n",
    });
    // The terminal exposes its accessible buffer on request; this also exercises its real parser.
    const textarea = document.querySelector(".terminal-pane textarea");
    textarea.focus();
    await new Promise((resolve) => requestAnimationFrame(resolve));
    textarea.dispatchEvent(
      new KeyboardEvent("keydown", {
        key: "F",
        code: "KeyF",
        ctrlKey: true,
        shiftKey: true,
        bubbles: true,
      }),
    );
    await wait(() =>
      document.querySelector('[aria-label="Search terminal output"]'),
    );
    const search = document.querySelector(
      '[aria-label="Search terminal output"]',
    );
    Object.getOwnPropertyDescriptor(
      HTMLInputElement.prototype,
      "value",
    ).set.call(search, "LOMI_NATIVE_AFTER_THEME");
    search.dispatchEvent(new Event("input", { bubbles: true }));
    search.dispatchEvent(
      new KeyboardEvent("keydown", {
        key: "Enter",
        code: "Enter",
        bubbles: true,
      }),
    );
    await wait(() =>
      document.querySelector(".search-result")?.textContent.includes(" / "),
    );
    const after = await invoke("plugin_smoke_result", {
      stage: "inspect",
      data: null,
    });
    if (JSON.stringify(before.terminals) !== JSON.stringify(after.terminals))
      throw Error("A theme or tab switch replaced the shell PID");
    if (browserBefore.bounds[2] <= 0 || browserBefore.bounds[3] <= 0)
      throw Error("Native browser has invalid geometry");
    await invoke("plugin_smoke_result", {
      stage: "passed",
      data: {
        engine: navigator.userAgent,
        url: location.href,
        checks: [
          "import without trust blocked",
          "actual host lazy ESM/shared hooks/context",
          "relative chunk/CSS/image/Unicode path",
          "keyboard Dockview move retains state and shell PID",
          "native PTY input during plugin work",
          "Explorer follows mkdir, file bursts, renames, removals and collapsed folders without Git",
          "active-folder native watch",
          "main/settings light theme convergence",
          "native child browser resize/zoom geometry/overlay hide/restore",
          "browser input state retained and application IPC rejected",
          "shell PID retained across theme and tab switches",
        ],
        terminals: after.terminals,
        webglCanvasCount: canvasCount,
        browser: browserBefore,
      },
    });
  } catch (error) {
    await invoke("plugin_smoke_result", {
      stage: "failed",
      data: {
        phase,
        error: String(error),
        stack: error.stack ?? "",
        viewport: [innerWidth, innerHeight],
        native: await invoke("plugin_smoke_result", {
          stage: "inspect",
          data: null,
        }),
      },
    });
  }
})();
