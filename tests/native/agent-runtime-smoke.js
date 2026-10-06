(async () => {
  const internal = window.__TAURI_INTERNALS__;
  const original = internal.invoke.bind(internal);
  const failureFixture = SMOKE_FAILURE;
  const calls = [];
  const nativeFetch = window.fetch.bind(window);
  // Tauri protects its invoke property. Observe its real custom-protocol fetch
  // response without replacing handlers or manufacturing native results.
  window.fetch = async (url, options) => {
    const command = String(url).startsWith("ipc://localhost/")
      ? decodeURIComponent(String(url).slice("ipc://localhost/".length))
      : "";
    if (
      [
        "agent_task_send",
        "agent_task_continue",
        "agent_task_commit_switch",
      ].includes(command)
    )
      throw Error("Archive smoke unexpectedly requested native work");
    const result = await nativeFetch(url, options);
    if (
      [
        "agent_task_prepare_close",
        "agent_task_close_release",
        "agent_runtime_prepare_close",
        "agent_tasks_drain",
        "save_session",
        "request_quit",
      ].includes(command)
    ) {
      const entry = {
        command,
        args: JSON.parse(options?.body ?? "{}"),
        completed: result.headers.get("Tauri-Response") === "ok",
      };
      calls.push(entry);
      await original("plugin_smoke_result", { stage: "call", data: entry });
    }
    return result;
  };
  const invoke = original;
  const report = (stage, data) =>
    original("plugin_smoke_result", { stage, data });
  const wait = async (fn) => {
    for (let i = 0; i < 300; i++) {
      if (await fn()) return;
      await new Promise((r) => setTimeout(r, 100));
    }
    throw Error(
      "Native task smoke timeout: " + document.body.textContent.slice(-3500),
    );
  };
  const button = (text, scope = document) =>
    [...scope.querySelectorAll("button")].find(
      (b) => b.textContent.trim() === text,
    );
  const panes = () =>
    [
      ...document.querySelectorAll(
        '.agent-task-pane[data-task-id="offline-archive-fixture"]',
      ),
    ].filter((e) => e.getBoundingClientRect().width);
  const references = (session) => {
    let count = 0;
    const walk = (value) => {
      if (!value || typeof value !== "object") return;
      if (
        value.type === "agent-task" &&
        value.taskId === "offline-archive-fixture"
      )
        count++;
      for (const child of Object.values(value)) walk(child);
    };
    walk(session);
    return count;
  };
  const savedRefs = async () => references(await invoke("load_session"));
  const command = async (label) => {
    window.dispatchEvent(
      new KeyboardEvent("keydown", {
        key: "P",
        code: "KeyP",
        metaKey: true,
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
  let phase = "restore";
  try {
    await wait(() =>
      panes()[0]?.textContent.includes("Preserved original partial output"),
    );
    if ((await savedRefs()) !== 2)
      throw Error("Restore lost shared task references");
    const state = await report("inspect", null);
    if (state.identifier !== "dev.lomi.agent-runtime-production-smoke-20261006")
      throw Error("Native smoke identifier was not isolated");
    if (!state.restored) {
      await report("screenshot", "restored");
      await report("restored", {
        pid: state.pid,
        taskReferences: 2,
        engine: navigator.userAgent,
      });
      location.reload();
      return;
    }
    if (!failureFixture) {
      phase = "owned production entry";
      const owned = await report("owned-production-entry", null);
      if (!owned.passed || owned.inference || owned.credentials)
        throw Error("Owned production entry fixture differs");
    }
    phase = "Dockview move";
    await command("Dock current tab");
    await wait(() => button("Dock"));
    button("Dock").click();
    await wait(() =>
      document.querySelector(".dock-pane-host .agent-task-pane"),
    );
    await wait(async () => (await savedRefs()) === 2);
    if (calls.some((c) => c.command === "agent_task_prepare_close"))
      throw Error("Dockview movement drained shared task");
    await report("screenshot", "docked");
    phase = "first view close";
    button("Close task", panes()[0]).click();
    await wait(async () => (await savedRefs()) === 1);
    if (calls.some((c) => c.command === "agent_task_prepare_close"))
      throw Error("Non-final view close drained task");
    phase = "final view close";
    document.querySelector('[data-tab-id="task-b"] [role="tab"]').click();
    await wait(() =>
      panes()[0]?.textContent.includes("Preserved original partial output"),
    );
    button("Close task", panes()[0]).click();
    if (failureFixture) {
      await wait(() =>
        document.body.textContent.includes("Views could not be closed"),
      );
      if ((await savedRefs()) !== 1 || !panes().length)
        throw Error("Failed native close removed final view");
      button("Keep open", document.querySelector('[role="dialog"]')).click();
      await invoke("request_quit");
      await wait(
        () =>
          button("Quit anyway") ||
          document.body.textContent.includes("Could not close the window"),
      );
      button("Quit anyway")?.click();
      await wait(() =>
        document.body.textContent.includes("Could not close the window"),
      );
      if ((await savedRefs()) !== 1 || !panes().length)
        throw Error("Failed native quit removed protected task");
      const closes = calls.filter(
        (c) => c.command === "agent_task_prepare_close",
      );
      const drains = calls.filter((c) => c.command === "agent_tasks_drain");
      if (
        closes.length !== 1 ||
        closes[0].completed !== false ||
        !drains.length ||
        drains.some((c) => c.completed !== false)
      )
        throw Error(
          "Retention did not observe rejected native close and drain",
        );
      await report("screenshot", "final-close");
      await report("retention-passed", {
        pid: state.pid,
        taskReferences: 1,
        checks: [
          "same-boot unknown ownership blocks final-view close",
          "failed close retains final task view and saved reference",
          "normal guarded quit refuses unresolved ownership and leaves own app alive",
        ],
        calls,
      });
      return;
    }
    await wait(async () => (await savedRefs()) === 0);
    await wait(() =>
      calls.some(
        (c) => c.command === "agent_task_close_release" && c.completed === true,
      ),
    );
    const closes = calls.filter(
      (c) => c.command === "agent_task_prepare_close",
    );
    if (
      closes.length !== 1 ||
      closes[0].completed !== true ||
      JSON.stringify(closes[0].args.taskIds) !== '["offline-archive-fixture"]'
    )
      throw Error("Final close did not drain exactly one native task");
    await report("screenshot", "final-close");
    await report("ready-to-quit", {
      pid: state.pid,
      engine: navigator.userAgent,
      taskReferences: 0,
      checks: [
        "shared archived task restored in real WebKit twice",
        "Dockview move retains two references without drain",
        "first close retains shared owner",
        "final close drains once and releases",
        "production guarded quit requested",
      ],
      calls,
    });
    phase = "normal quit";
    await invoke("request_quit");
    for (let i = 0; i < 100; i++) {
      button("Quit anyway")?.click();
      await new Promise((r) => setTimeout(r, 100));
    }
  } catch (error) {
    await report("failed", {
      phase,
      error: String(error),
      stack: error.stack,
      calls,
    });
  }
})();
