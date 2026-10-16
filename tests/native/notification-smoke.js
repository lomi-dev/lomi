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
    const notificationState = await invoke("desktop_notifications_get_state");
    if (!notificationState.available || !notificationState.preferences.enabled)
      throw Error(
        "Native notification smoke requires a verified account session with desktop notifications enabled; its legacy isolated signed-out fixture cannot validate delivery.",
      );
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
    const inboxOnly =
      new URLSearchParams(location.search).get("notification-smoke") ===
      "inbox";
    if (!inboxOnly) {
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
          document.querySelector(".terminal-activity")?.textContent ===
          "Working",
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
          await wait(
            () =>
              document
                .querySelector('[data-tab-id="settings"] [role="tab"]')
                ?.getAttribute("aria-selected") === "true",
          );
        } else {
          document.querySelector('[aria-label="Close Settings"]')?.click();
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
    }
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
    await invoke("plugin_smoke_result", {
      stage: "notification-foreground",
      data: null,
    });
    await wait(() => document.hasFocus());
    checkpoint = "foreground signal";
    await signal("finished");
    await wait(() => notices.length === 1);
    if (notices[0].requested) throw Error("Focused main window sent an alert");
    const foregroundInbox = await invoke("load_notifications");
    if (
      foregroundInbox.items.length !== 1 ||
      foregroundInbox.items[0].read ||
      foregroundInbox.items[0].agent !== "claude" ||
      foregroundInbox.items[0].title !== "Claude Code finished responding"
    )
      throw Error("Focused notification was not saved unread in the inbox");
    if (document.querySelector(".notification-center, [role=alert]"))
      throw Error("Receipt opened an in-app notification or alert");

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
    if ((await invoke("load_notifications")).items.length !== 2)
      throw Error("Background notification was not saved in the inbox");
    await signal("attention");
    await pause(300);
    if (notices.length !== 2) throw Error("Duplicate alert was delivered");
    if ((await invoke("load_notifications")).items.length !== 2)
      throw Error("Duplicate notification was stored in the inbox");

    await invoke("plugin_smoke_result", {
      stage: "notification-toggle",
      data: false,
    });
    await wait(
      async () =>
        (await invoke("desktop_notifications_get_state"))?.preferences
          .enabled === false,
    );
    await pause(100);
    await pause(2100);
    await signal("finished");
    await pause(300);
    if (notices.some((notice, index) => index >= 2 && notice.requested))
      throw Error("Disabled alerts were delivered");
    if ((await invoke("load_notifications")).items.length !== 2)
      throw Error("Disabled notification was stored in the inbox");
    checkpoint = "re-enable";
    const quietCount = notices.length;
    await invoke("plugin_smoke_result", {
      stage: "notification-toggle",
      data: true,
    });
    await wait(
      async () =>
        (await invoke("desktop_notifications_get_state"))?.preferences
          .enabled === true,
    );
    await pause(2100);
    await signal("finished");
    await wait(() => notices.length > quietCount);
    if (!notices.at(-1).requested)
      throw Error("Re-enabled alert was not requested");
    checkpoint = "inbox read state and account menu";
    const incomingInbox = await invoke("load_notifications");
    if (incomingInbox.items.length !== 3)
      throw Error("Re-enabled notification was not saved in the inbox");
    const firstRead = await invoke("mark_notifications_read", {
      ids: [foregroundInbox.items[0].id],
    });
    if (
      firstRead.items.filter((item) => item.read).length !== 1 ||
      firstRead.items.filter((item) => !item.read).length !== 2
    )
      throw Error("Reading an old notification marked newer arrivals read");
    if (
      JSON.stringify(await invoke("load_notifications")) !==
      JSON.stringify(firstRead)
    )
      throw Error("Inbox did not restore saved read state");
    await invoke("plugin_smoke_result", {
      stage: "notification-foreground",
      data: null,
    });
    await wait(() => document.hasFocus());
    document.querySelector('button[aria-label="Open account menu"]').click();
    await wait(() =>
      document.querySelector('[role="menuitem"][aria-label="Notifications"]'),
    );
    document
      .querySelector('[role="menuitem"][aria-label="Notifications"]')
      .click();
    await wait(() => document.querySelector(".notification-center"));
    if (
      !document
        .querySelector(".notification-center")
        .textContent.includes("Claude Code finished responding")
    )
      throw Error("Account menu inbox did not display native notifications");
    if (
      !document
        .querySelector(".notification-center")
        .textContent.includes("Notification from Claude Code")
    )
      throw Error("Native inbox did not display persisted agent attribution");
    const clearButton = [
      ...document.querySelectorAll(".notification-center button"),
    ].find((button) => button.textContent.trim() === "Clear all notifications");
    if (!clearButton || clearButton.disabled)
      throw Error(
        "Clear all notifications was not available for a mixed inbox",
      );
    clearButton.click();
    await wait(() =>
      document
        .querySelector(".notification-center")
        .textContent.includes("No notifications"),
    );
    const cleared = await invoke("load_notifications");
    if (cleared.items.length !== 0 || !clearButton.disabled)
      throw Error(
        "Clearing the mixed inbox did not persist or disable the empty action",
      );
    document.querySelector('button[aria-label="Close notifications"]').click();
    await wait(() => !document.querySelector(".notification-center"));
    checkpoint = "verified foreground CLI attribution";
    const beforeAgent = notices.length;
    await invoke("write_terminal", { id, data: "./codex\r" });
    await wait(() => notices.length === beforeAgent + 1);
    const attributedInbox = await invoke("load_notifications");
    if (
      attributedInbox.items.length !== 1 ||
      attributedInbox.items[0].agent !== "codex" ||
      attributedInbox.items[0].title !== "Agent needs your input" ||
      JSON.stringify(attributedInbox).includes("PRIVATE_NATIVE_AGENT_MESSAGE")
    )
      throw Error("Foreground CLI OSC 9 attribution was not saved privately");
    await invoke("write_terminal", { id, data: "\u0003" });
    await wait(async () => !(await invoke("terminal_contexts"))[id]?.titleCli);
    if ((await invoke("load_notifications")).items[0].agent !== "codex")
      throw Error("Process exit changed persisted notification attribution");
    document.querySelector('button[aria-label="Open account menu"]').click();
    await wait(() =>
      document.querySelector('[role="menuitem"][aria-label="Notifications"]'),
    );
    document
      .querySelector('[role="menuitem"][aria-label="Notifications"]')
      .click();
    await wait(() => document.querySelector(".notification-center"));
    const unreadClear = [
      ...document.querySelectorAll(".notification-center button"),
    ].find((button) => button.textContent.trim() === "Clear all notifications");
    if (!unreadClear || unreadClear.disabled)
      throw Error(
        "Clear all notifications was disabled for an unread-only inbox",
      );
    unreadClear.click();
    await wait(
      async () => (await invoke("load_notifications")).items.length === 0,
    );
    document.querySelector('button[aria-label="Close notifications"]').click();
    await wait(() => !document.querySelector(".notification-center"));
    checkpoint = "unknown terminal attribution";
    const beforeUnknown = notices.length;
    let cloudQualificationError = "";
    try {
      await invoke("notify_agent", {
        kind: "attention",
        source: "terminal",
        sessionId: "nonexistent-terminal-session",
        notificationId: crypto.randomUUID(),
        context: "Unknown terminal source",
      });
    } catch (error) {
      cloudQualificationError = String(error);
    }
    if (
      !cloudQualificationError.includes("terminal session is no longer running")
    )
      throw Error(
        "Unknown terminal session was not rejected for cloud forwarding",
      );
    await wait(() => notices.length === beforeUnknown + 1);
    const unknownInbox = await invoke("load_notifications");
    if (
      unknownInbox.items.length !== 1 ||
      unknownInbox.items[0].agent != null ||
      unknownInbox.items[0].title !== "Agent needs your input"
    )
      throw Error(
        "Rejected cloud qualification suppressed the local inbox or changed attribution",
      );
    await invoke("dismiss_notification", { id: unknownInbox.items[0].id });
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
    checkpoint = "system alerts survive inbox storage failure";
    await invoke("plugin_smoke_result", {
      stage: "notification-background",
      data: null,
    });
    await invoke("plugin_smoke_result", {
      stage: "notification-inbox-corrupt",
      data: null,
    });
    const beforeFailure = notices.length;
    let storageError = "";
    try {
      await invoke("notify_agent", {
        kind: "finished",
        source: "claude",
        context: "Isolated storage failure test",
      });
    } catch (error) {
      storageError = String(error);
    }
    if (!storageError.includes("left intact"))
      throw Error("Inbox storage failure was not reported");
    await wait(() => notices.length === beforeFailure + 1);
    if (!notices.at(-1).requested)
      throw Error("Inbox storage failure suppressed the system alert request");
    await invoke("plugin_smoke_result", {
      stage: "notification-inbox-preserved",
      data: null,
    });
    await invoke("plugin_smoke_result", {
      stage: "passed",
      data: {
        checks: [
          "isolated Claude hook installation",
          "real PTY OSC parsing",
          ...(!inboxOnly
            ? [
                "single-terminal title and work indicator without maximization",
                "activity cleared on the native shell prompt",
                "native quit during renderer startup preserves the main window",
                "native Cmd+Q, AppKit Quit and window close confirm active terminal work",
                "Cmd+Q with terminal or Settings focused protects vim and its unsaved input",
                "repeated quit requests share one dialog with Cancel focused",
                "cancelling native quit preserves the running process and PTY",
              ]
            : []),
          "foreground suppression",
          "focused and background notifications persisted without in-app alerts",
          "explicit reads preserve newer unread arrivals",
          "native account menu opens the persisted inbox",
          "persisted Claude attribution and generic fallback for unknown sessions",
          "real PTY OSC 9 preserves verified foreground CLI identity after process exit",
          "read state reload and native clear-all button for mixed and unread-only inboxes",
          "storage failures preserve inbox and still request system alerts",
          "native notification request in background",
          "duplicate suppression",
          "Settings tab toggle applied live",
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
