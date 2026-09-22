(async () => {
  const invoke = window.__TAURI_INTERNALS__.invoke;
  const notices = [];
  let checkpoint = "initializing";
  const pause = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
  const wait = async (check) => {
    for (let i = 0; i < 200; i++) {
      if (await check()) return;
      await pause(100);
    }
    throw Error("Native notification check timed out");
  };
  try {
    const { listen } = await import("/node_modules/@tauri-apps/api/event.js");
    await listen("notification-smoke-result", ({ payload }) =>
      notices.push(payload),
    );
    const { runningTerminal } = await import("/src/terminal-runtime.ts");
    let runtime;
    await wait(
      () =>
        (runtime = runningTerminal("notification-terminal"))?.getSnapshot()
          .status === "running",
    );
    const id = runtime.sessionId;
    checkpoint = "terminal activity title";
    await invoke("plugin_smoke_result", {
      stage: "notification-foreground",
      data: null,
    });
    await wait(() => runtime.getSnapshot().cwd && document.hasFocus());
    await invoke("write_terminal", {
      id,
      data: "printf '\\033]2;⠋ Native agent task\\007\\033]777;notify;Lomi;claude;working\\007'; sleep 3\r",
    });
    await wait(
      () =>
        document.querySelector(".terminal-activity")?.textContent === "Working",
    );
    if (
      document.querySelector(".terminal-title")?.textContent !==
      "Native agent task"
    )
      throw Error("Single terminal did not display its title");
    if (document.querySelector(".terminal-heading button"))
      throw Error("Single terminal offered maximization");
    if (document.querySelector(".terminal-title-box svg"))
      throw Error("Single terminal still has a terminal icon");
    await wait(() => runtime.getSnapshot().agentSignal === null);
    if (document.querySelector(".terminal-activity"))
      throw Error("Activity remained visible after the shell prompt");
    checkpoint = "native agent quit confirmation";
    await invoke("write_terminal", {
      id,
      data: "printf '\\033]777;notify;Lomi;claude;working\\007'; sleep 30\r",
    });
    await wait(async () =>
      (await invoke("busy_terminals", { ids: [id] })).includes(id),
    );
    for (const stage of [
      "notification-cmd-q",
      "notification-quit",
      "notification-close",
    ]) {
      checkpoint = stage;
      await invoke("plugin_smoke_result", { stage, data: null });
      await wait(
        () =>
          document.querySelector("dialog[open] h2")?.textContent ===
          "Quit Lomi?",
      );
      const dialog = document.querySelector("dialog[open]");
      const cancel = [...dialog.querySelectorAll("button")].find(
        (button) => button.textContent === "Cancel",
      );
      if (document.activeElement !== cancel)
        throw Error("Quit confirmation did not focus Cancel");
      if (!dialog.textContent.includes("running processes"))
        throw Error(
          "Quit confirmation did not detect the real terminal process",
        );
      await invoke("plugin_smoke_result", { stage, data: null });
      await pause(100);
      if (document.querySelectorAll("dialog[open]").length !== 1)
        throw Error("Repeated native quit opened multiple dialogs");
      cancel.click();
      await wait(() => !document.querySelector("dialog[open]"));
      await pause(100);
      if (!(await invoke("busy_terminals", { ids: [id] })).includes(id))
        throw Error("Cancelling quit stopped the terminal process");
      if (runningTerminal("notification-terminal")?.sessionId !== id)
        throw Error("Cancelling quit replaced the PTY");
    }
    await invoke("write_terminal", { id, data: "\u0003" });
    await wait(() => runtime.getSnapshot().agentSignal === null);
    checkpoint = "native quit with an interactive terminal program";
    await invoke("write_terminal", {
      id,
      data: "/usr/bin/vim -Nu NONE -n -i NONE quit-guard.txt\r",
    });
    await wait(() => runtime.terminal.buffer.active.type === "alternate");
    await invoke("write_terminal", { id, data: "iUNSAVED QUIT GUARD" });
    const vimText = () =>
      Array.from({ length: runtime.terminal.buffer.active.length }, (_, i) =>
        runtime.terminal.buffer.active.getLine(i)?.translateToString(),
      ).join("\n");
    await wait(() => vimText().includes("UNSAVED QUIT GUARD"));
    for (const focus of ["terminal", "settings"]) {
      checkpoint = `native Cmd+Q with vim and ${focus} focused`;
      if (focus === "settings") {
        await invoke("plugin_smoke_result", {
          stage: "notification-settings",
          data: null,
        });
        await wait(() => !document.hasFocus());
      } else {
        runtime.terminal.focus();
      }
      await invoke("plugin_smoke_result", {
        stage: "notification-cmd-q",
        data: null,
      });
      await wait(
        () =>
          document.querySelector("dialog[open] h2")?.textContent ===
          "Quit Lomi?",
      );
      const dialog = document.querySelector("dialog[open]");
      const cancel = [...dialog.querySelectorAll("button")].find(
        (button) => button.textContent === "Cancel",
      );
      if (!dialog.textContent.includes("running processes"))
        throw Error("Quit did not detect the interactive terminal program");
      if (document.activeElement !== cancel)
        throw Error("Cmd+Q did not focus Cancel");
      cancel.click();
      await wait(() => !document.querySelector("dialog[open]"));
      if (
        runtime.terminal.buffer.active.type !== "alternate" ||
        !vimText().includes("UNSAVED QUIT GUARD") ||
        runningTerminal("notification-terminal")?.sessionId !== id ||
        !(await invoke("busy_terminals", { ids: [id] })).includes(id)
      )
        throw Error(
          "Cancelling Cmd+Q lost the running vim session or its input",
        );
    }
    await invoke("write_terminal", { id, data: "\u001b:q!\r" });
    await wait(() => runtime.terminal.buffer.active.type === "normal");
    const configuration = await invoke("inspect_agent_notifications");
    if (!configuration.path.includes("lomi-notification-native-"))
      throw Error("Configuration is not isolated");
    await invoke("enable_agent_notifications", {
      path: configuration.path,
      revision: configuration.revision,
    });
    if (!(await invoke("inspect_agent_notifications")).configured)
      throw Error("Hooks were not installed");
    await invoke("plugin:notification|request_permission");
    const signal = async (kind) => {
      await invoke("write_terminal", {
        id,
        data: `printf '\\033]777;notify;Lomi;claude;${kind}\\007'\r`,
      });
    };
    checkpoint = "foreground focus";
    await wait(() => document.hasFocus());
    checkpoint = "foreground signal";
    await signal("finished");
    await wait(() => notices.length === 1);
    if (notices[0].requested) throw Error("Focused main window sent an alert");

    // Hide the native window, keeping its PTY and xterm parser alive.
    await invoke("plugin_smoke_result", {
      stage: "notification-background",
      data: null,
    });
    checkpoint = "background signal";
    await signal("attention");
    await wait(() => notices.length === 2);
    if (!notices[1].requested)
      throw Error("Background alert was not requested");
    await signal("attention");
    await pause(300);
    if (notices.length !== 2) throw Error("Duplicate alert was delivered");

    await invoke("plugin_smoke_result", {
      stage: "notification-toggle",
      data: false,
    });
    await wait(
      async () =>
        (await invoke("load_terminal_preferences"))?.agentNotifications ===
        false,
    );
    await pause(100);
    await pause(2100);
    await signal("finished");
    await pause(300);
    if (notices.some((notice, index) => index >= 2 && notice.requested))
      throw Error("Disabled alerts were delivered");
    checkpoint = "re-enable";
    const quietCount = notices.length;
    await invoke("plugin_smoke_result", {
      stage: "notification-toggle",
      data: true,
    });
    await wait(
      async () =>
        (await invoke("load_terminal_preferences"))?.agentNotifications ===
        true,
    );
    await pause(2100);
    await signal("finished");
    await wait(() => notices.length > quietCount);
    if (!notices.at(-1).requested)
      throw Error("Re-enabled alert was not requested");
    if (runningTerminal("notification-terminal")?.sessionId !== id)
      throw Error("Preferences restarted the PTY");
    await invoke("write_terminal", {
      id,
      data: "printf 'NOTIFICATION_NATIVE_OK\\n'\r",
    });
    await wait(() =>
      Array.from({ length: runtime.terminal.buffer.active.length }, (_, i) =>
        runtime.terminal.buffer.active.getLine(i)?.translateToString(),
      )
        .join("\n")
        .includes("NOTIFICATION_NATIVE_OK"),
    );
    await invoke("plugin_smoke_result", {
      stage: "passed",
      data: {
        checks: [
          "isolated Claude hook installation",
          "real PTY OSC parsing",
          "single-terminal title and work indicator without maximization",
          "activity cleared on the native shell prompt",
          "native quit during renderer startup preserves the main window",
          "native Cmd+Q, AppKit Quit and window close confirm active terminal work",
          "Cmd+Q with terminal or Settings focused protects vim and its unsaved input",
          "repeated quit requests share one dialog with Cancel focused",
          "cancelling native quit preserves the running process and PTY",
          "foreground suppression",
          "native notification request in background",
          "duplicate suppression",
          "settings-window toggle applied live",
          "PTY retained after preference changes",
        ],
        notices,
      },
    });
  } catch (error) {
    await invoke("plugin_smoke_result", {
      stage: "failed",
      data: {
        error: String(error),
        checkpoint,
        notices,
        focused: document.hasFocus(),
        errors: [...document.querySelectorAll("[role=alert]")].map(
          (node) => node.textContent,
        ),
      },
    });
  }
})();
