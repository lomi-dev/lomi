(async () => {
  const invoke = window.__TAURI_INTERNALS__.invoke;
  const directory = SMOKE_DIRECTORY;
  const agyExecutable = SMOKE_AGY_EXECUTABLE;
  const agyHome = SMOKE_AGY_HOME;
  const liveHome = SMOKE_LIVE_HOME;
  const liveKimiHome = SMOKE_LIVE_KIMI_HOME;
  const liveAgyExecutable = SMOKE_LIVE_AGY_EXECUTABLE;
  const liveAgyHome = SMOKE_LIVE_AGY_HOME;
  const liveMode = SMOKE_LIVE_MODE;
  const liveProviders = [];
  const offlineProviders = [];
  let checkpoint = "initializing";
  const pause = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
  const wait = async (check) => {
    for (let i = 0; i < 250; i++) {
      if (await check()) return;
      await pause(100);
    }
    throw Error("Native usage check timed out");
  };
  try {
    const { runningTerminal } = await import("/src/terminal-runtime.ts");
    let runtime;
    await wait(
      () =>
        (runtime = runningTerminal("usage-terminal"))?.getSnapshot().status ===
        "running",
    );
    const id = runtime.sessionId;
    const waitPrompt = () =>
      wait(() => runtime.atPrompt && !runtime.activeBlock);
    const empty = await invoke("inspect_cli_usage", {
      targets: [],
      force: false,
    });
    if (empty.entries.length) throw Error("Idle usage returned agents");
    checkpoint = "initial shell prompt";
    try {
      await waitPrompt();
    } catch {
      const context = (await invoke("terminal_contexts"))[id];
      const terminalText = Array.from(
        { length: runtime.terminal.buffer.active.length },
        (_, i) =>
          runtime.terminal.buffer.active.getLine(i)?.translateToString(),
      ).join("\n");
      throw Error(
        JSON.stringify({
          reason: "Initial shell prompt was not parsed",
          terminal: {
            status: runtime.getSnapshot().status,
            atPrompt: Boolean(runtime.atPrompt),
            activeBlock: Boolean(runtime.activeBlock),
            receivedOutput: Boolean(runtime.receivedOutput),
            commandFailure: /command not found|permission denied|killed/i.test(
              terminalText,
            ),
          },
          nativeContext: {
            foregroundProgram: context?.foregroundProgram ?? null,
            titleCli: context?.titleCli?.cli ?? null,
            titlePid: context?.titleCli?.pid ?? null,
          },
        }),
      );
    }
    const quote = (value) => `'${value.replaceAll("'", "'\\''")}'`;
    checkpoint = "native agent detection";
    await invoke("write_terminal", {
      id,
      data: `/usr/bin/env -u OPENAI_API_KEY -u OPENAI_BASE_URL -u OPENAI_API_BASE -u CODEX_API_KEY CODEX_HOME=${quote(directory + "/codex-home")} ${quote(directory + "/codex")} 120\r`,
    });
    let process;
    try {
      await wait(async () => {
        process = (await invoke("terminal_contexts"))[id]?.titleCli;
        return process?.cli === "codex";
      });
    } catch {
      const context = (await invoke("terminal_contexts"))[id];
      const terminalText = Array.from(
        { length: runtime.terminal.buffer.active.length },
        (_, i) =>
          runtime.terminal.buffer.active.getLine(i)?.translateToString(),
      ).join("\n");
      throw Error(
        JSON.stringify({
          reason: "Native agent detection timed out",
          terminal: {
            atPrompt: Boolean(runtime.atPrompt),
            activeBlock: Boolean(runtime.activeBlock),
            commandEchoed: terminalText.includes(directory + "/codex"),
            commandFailure: /command not found|permission denied|killed/i.test(
              terminalText,
            ),
          },
          nativeContext: {
            foregroundProgram: context?.foregroundProgram ?? null,
            titleCli: context?.titleCli?.cli ?? null,
            titlePid: context?.titleCli?.pid ?? null,
          },
        }),
      );
    }
    const target = { id, process };
    const snapshot = await invoke("inspect_cli_usage", {
      targets: [target],
      force: false,
    });
    const codexEntry = snapshot.entries.find(
      (entry) => entry.id === id && entry.process?.pid === process.pid,
    );
    if (codexEntry?.status !== "unauthenticated")
      throw Error(
        "An isolated agent without credentials did not report missing authentication",
      );
    if (codexEntry.windows.length)
      throw Error("Missing authentication produced invented usage");
    offlineProviders.push({
      cli: "codex",
      status: codexEntry.status,
      numericWindows: codexEntry.windows.length,
    });
    await wait(() => document.querySelector(".agent-usage-trigger"));
    checkpoint = "native details menu";
    document.querySelector(".agent-usage-trigger").click();
    await wait(() => document.querySelector(".agent-usage-menu"));
    if (
      !document.querySelector(".agent-usage-menu").textContent.includes("Codex")
    )
      throw Error("Usage details did not identify the running agent");
    document
      .querySelector(".agent-usage-menu")
      .dispatchEvent(
        new KeyboardEvent("keydown", { key: "Escape", bubbles: true }),
      );
    await wait(() => !document.querySelector(".agent-usage-menu"));
    if (runningTerminal("usage-terminal")?.sessionId !== id)
      throw Error("Inspecting usage replaced the PTY");
    checkpoint = "process replacement";
    await invoke("write_terminal", { id, data: "\u0003" });
    await wait(async () => !(await invoke("terminal_contexts"))[id]?.titleCli);
    await waitPrompt();
    const stopped = await invoke("inspect_cli_usage", {
      targets: [target],
      force: true,
    });
    if (stopped.entries.some((entry) => entry.status === "ready"))
      throw Error("Stopped process received a cached usage result");
    await wait(() => !document.querySelector(".agent-usage-trigger"));
    checkpoint = "Kimi offline credentials and process ownership";
    await invoke("write_terminal", {
      id,
      data: `KIMI_CODE_HOME=${quote(directory + "/kimi-home")} ${quote(directory + "/kimi")} 120\r`,
    });
    let kimiProcess;
    await wait(async () => {
      kimiProcess = (await invoke("terminal_contexts"))[id]?.titleCli;
      return kimiProcess?.cli === "kimi";
    });
    if (!Number.isSafeInteger(kimiProcess.pid) || kimiProcess.pid <= 0)
      throw Error("Kimi detection did not include the running CLI PID");
    const kimiTarget = { id, process: kimiProcess };
    const kimiSnapshot = await invoke("inspect_cli_usage", {
      targets: [kimiTarget],
      force: false,
    });
    const kimiEntry = kimiSnapshot.entries.find(
      (entry) => entry.id === id && entry.process?.cli === "kimi",
    );
    if (kimiEntry?.process?.pid !== kimiProcess.pid)
      throw Error("Kimi usage result lost ownership of the detected CLI PID");
    if (kimiEntry.status !== "unauthenticated")
      throw Error(
        "Kimi with an empty isolated KIMI_CODE_HOME did not report missing authentication",
      );
    if (kimiEntry.windows.length)
      throw Error("Missing Kimi authentication produced invented usage");
    await wait(() => document.querySelector(".agent-usage-trigger"));
    document.querySelector(".agent-usage-trigger").click();
    await wait(() => document.querySelector(".agent-usage-menu"));
    if (
      !document
        .querySelector(".agent-usage-menu")
        .textContent.includes("Kimi Code CLI")
    )
      throw Error("Kimi usage details did not identify the running agent");
    document
      .querySelector(".agent-usage-menu")
      .dispatchEvent(
        new KeyboardEvent("keydown", { key: "Escape", bubbles: true }),
      );
    await wait(() => !document.querySelector(".agent-usage-menu"));
    await invoke("write_terminal", { id, data: "\u0003" });
    await wait(async () => !(await invoke("terminal_contexts"))[id]?.titleCli);
    await waitPrompt();
    const stoppedKimi = await invoke("inspect_cli_usage", {
      targets: [kimiTarget],
      force: true,
    });
    const stoppedKimiEntry = stoppedKimi.entries.find(
      (entry) => entry.id === id,
    );
    if (
      stoppedKimiEntry?.status !== "error" ||
      stoppedKimiEntry.process?.cli !== "kimi" ||
      stoppedKimiEntry.process?.pid !== kimiProcess.pid ||
      !stoppedKimiEntry.message?.includes("no longer running")
    )
      throw Error(
        "Stopped Kimi did not preserve its PID and report its stopped state",
      );
    await wait(() => !document.querySelector(".agent-usage-trigger"));
    offlineProviders.push({
      cli: "kimi",
      status: kimiEntry.status,
      numericWindows: kimiEntry.windows.length,
      processPidOwned:
        kimiEntry.process.cli === "kimi" &&
        kimiEntry.process.pid === kimiProcess.pid,
      stoppedStatus: stoppedKimiEntry.status,
      stoppedProcessPidOwned:
        stoppedKimiEntry.process.cli === "kimi" &&
        stoppedKimiEntry.process.pid === kimiProcess.pid,
      stoppedMessage: stoppedKimiEntry.message,
    });

    checkpoint = "Antigravity native usage fixture";
    await invoke("write_terminal", {
      id,
      data: `PATH=${quote(directory + "/agy-bin")}:$PATH HOME=${quote(agyHome)} ${quote(agyExecutable)} 120\r`,
    });
    let agyProcess;
    await wait(async () => {
      agyProcess = (await invoke("terminal_contexts"))[id]?.titleCli;
      return agyProcess?.cli === "agy";
    });
    if (!Number.isSafeInteger(agyProcess.pid) || agyProcess.pid <= 0)
      throw Error("Antigravity detection did not include the running CLI PID");
    const agyTarget = { id, process: agyProcess };
    const agySnapshot = await invoke("inspect_cli_usage", {
      targets: [agyTarget],
      force: false,
    });
    const agyEntry = agySnapshot.entries.find(
      (entry) => entry.id === id && entry.process?.pid === agyProcess.pid,
    );
    const expectedAgyReset = Date.parse("2099-01-01T00:00:00Z");
    const hasAgyPercent = (entry, expected) =>
      entry?.windows.some(
        (usageWindow) =>
          typeof usageWindow.remainingPercent === "number" &&
          Math.abs(usageWindow.remainingPercent - expected) < 1e-8,
      );
    const agyResetFieldVerified =
      Array.isArray(agyEntry?.windows) &&
      agyEntry.windows.length === 2 &&
      agyEntry.windows.every(
        (usageWindow) => usageWindow.resetsAt === expectedAgyReset,
      );
    const agyNumericWindows = (agyEntry?.windows ?? []).filter(
      (usageWindow) =>
        typeof usageWindow.remainingPercent === "number" &&
        Number.isFinite(usageWindow.remainingPercent),
    ).length;
    if (
      agyEntry?.status !== "ready" ||
      agyEntry.process?.cli !== "agy" ||
      agyNumericWindows !== 2 ||
      !hasAgyPercent(agyEntry, 81) ||
      !hasAgyPercent(agyEntry, 44) ||
      !agyResetFieldVerified
    )
      throw Error(
        "Antigravity's exact readonly usage command did not return the isolated quota fixture",
      );
    await wait(() => document.querySelector(".agent-usage-trigger"));
    document.querySelector(".agent-usage-trigger").click();
    await wait(() => document.querySelector(".agent-usage-menu"));
    const agyMenu = document.querySelector(".agent-usage-menu");
    if (
      document
        .querySelector(".agent-usage-trigger-value")
        ?.textContent.trim() !== "44%" ||
      !agyMenu.textContent.includes("Antigravity CLI") ||
      !agyMenu.textContent.includes("44% remaining")
    )
      throw Error("Antigravity usage details did not show its quota fixture");

    await invoke("plugin_smoke_result", {
      stage: "agy-fixture-decline",
      data: {},
    });
    await wait(() =>
      document
        .querySelector(".agent-usage-menu")
        ?.textContent.includes("7% remaining"),
    );
    if (
      document
        .querySelector(".agent-usage-trigger-value")
        ?.textContent.trim() !== "7%"
    )
      throw Error(
        "Antigravity titlebar usage did not show the declined quota fixture",
      );
    const declinedAgyEntry = await invoke("inspect_cli_usage", {
      targets: [agyTarget],
      force: false,
    }).then((snapshot) =>
      snapshot.entries.find(
        (entry) => entry.id === id && entry.process?.pid === agyProcess.pid,
      ),
    );
    if (
      declinedAgyEntry?.status !== "ready" ||
      declinedAgyEntry.windows.length !== 2 ||
      !hasAgyPercent(declinedAgyEntry, 12) ||
      !hasAgyPercent(declinedAgyEntry, 7) ||
      !declinedAgyEntry.windows.every(
        (usageWindow) => usageWindow.resetsAt === expectedAgyReset,
      )
    )
      throw Error("Antigravity refresh did not use the changed quota fixture");
    document
      .querySelector(".agent-usage-menu")
      .dispatchEvent(
        new KeyboardEvent("keydown", { key: "Escape", bubbles: true }),
      );
    await wait(() => !document.querySelector(".agent-usage-menu"));
    await invoke("write_terminal", { id, data: "\u0003" });
    await wait(async () => !(await invoke("terminal_contexts"))[id]?.titleCli);
    const stoppedAgy = await invoke("inspect_cli_usage", {
      targets: [agyTarget],
      force: true,
    });
    const stoppedAgyEntry = stoppedAgy.entries.find((entry) => entry.id === id);
    if (
      stoppedAgyEntry?.status !== "error" ||
      stoppedAgyEntry.process?.cli !== "agy" ||
      stoppedAgyEntry.process?.pid !== agyProcess.pid ||
      !stoppedAgyEntry.message?.includes("no longer running") ||
      stoppedAgyEntry.windows.length
    )
      throw Error(
        "Stopped Antigravity process did not reject its cached usage result",
      );
    await wait(() => !document.querySelector(".agent-usage-trigger"));
    await waitPrompt();
    offlineProviders.push({
      cli: "agy",
      status: agyEntry.status,
      numericWindows: agyNumericWindows,
      processPidOwned:
        agyEntry.process.cli === "agy" &&
        agyEntry.process.pid === agyProcess.pid,
      declinedRefreshVerified: declinedAgyEntry.status === "ready",
      resetFieldVerified: agyResetFieldVerified,
      declinedResetFieldVerified: declinedAgyEntry.windows.every(
        (usageWindow) => usageWindow.resetsAt === expectedAgyReset,
      ),
      menuVerified: true,
      stoppedStatus: stoppedAgyEntry.status,
      stoppedProcessPidOwned:
        stoppedAgyEntry.process.cli === "agy" &&
        stoppedAgyEntry.process.pid === agyProcess.pid,
    });

    if (liveMode) {
      const probeLiveProvider = async (
        cli,
        executable,
        commandPrefix = "",
        { includeMessage = true, verifyAgyTurnGuard = false } = {},
      ) => {
        checkpoint = `live account usage: ${cli}`;
        let status = "not-detected";
        let message = "The inert CLI fixture was not detected.";
        let numericWindows = 0;
        let launched = false;
        let liveProcess;
        try {
          await invoke("write_terminal", {
            id,
            data: `${commandPrefix}${quote(
              executable.startsWith("/")
                ? executable
                : directory + "/" + executable,
            )} 120\r`,
          });
          launched = true;
          try {
            await wait(async () => {
              liveProcess = (await invoke("terminal_contexts"))[id]?.titleCli;
              return liveProcess?.cli === cli;
            });
          } catch {
            status = "not-detected";
            message = "The inert CLI fixture was not detected.";
          }
          if (liveProcess?.cli === cli) {
            try {
              const live = await invoke("inspect_cli_usage", {
                targets: [{ id, process: liveProcess }],
                force: false,
              });
              const entry = live.entries.find(
                (candidate) =>
                  candidate.id === id &&
                  candidate.process?.cli === cli &&
                  candidate.process?.pid === liveProcess.pid,
              );
              status = entry?.status ?? "missing-entry";
              message = entry?.message ?? "";
              numericWindows = (entry?.windows ?? []).filter((usageWindow) =>
                [
                  usageWindow.remainingPercent,
                  usageWindow.used,
                  usageWindow.limit,
                ].some(
                  (value) =>
                    typeof value === "number" && Number.isFinite(value),
                ),
              ).length;
            } catch {
              status = "request-error";
              message = "The native account usage request could not complete.";
            }
          }
        } catch {
          status = "probe-error";
          message = "The inert CLI fixture could not be started.";
        } finally {
          if (launched) {
            await invoke("write_terminal", { id, data: "\u0003" });
            await wait(
              async () => !(await invoke("terminal_contexts"))[id]?.titleCli,
            );
            await wait(() => !document.querySelector(".agent-usage-trigger"));
            await waitPrompt();
          }
        }
        liveProviders.push({
          cli,
          status,
          numericWindows,
          readNumericWindows: numericWindows > 0,
          ...(includeMessage ? { message } : {}),
          ...(verifyAgyTurnGuard
            ? { zeroModelTurnGuardPassed: status === "ready" }
            : {}),
        });
      };

      if (liveMode === "codex" || liveMode === "all") {
        await probeLiveProvider(
          "codex",
          "codex",
          `CODEX_HOME=${quote(liveHome)} `,
        );
      }
      if (liveMode === "all") {
        await probeLiveProvider("claude", "claude");
        await probeLiveProvider("cursor", "cursor-agent");
        await probeLiveProvider(
          "kimi",
          "kimi",
          `KIMI_CODE_HOME=${quote(liveKimiHome)} `,
        );
      }
      if (liveMode === "agy") {
        if (
          typeof liveAgyExecutable !== "string" ||
          typeof liveAgyHome !== "string"
        )
          throw Error("The live Antigravity fixture was not prepared");
        await probeLiveProvider(
          "agy",
          liveAgyExecutable,
          `PATH=${quote(directory + "/live-agy-bin")}:$PATH HOME=${quote(liveAgyHome)} `,
          { includeMessage: false, verifyAgyTurnGuard: true },
        );
      }
    }
    checkpoint = "unsupported agent";
    await invoke("write_terminal", {
      id,
      data: `${quote(directory + "/copilot")} 120\r`,
    });
    await wait(async () => {
      process = (await invoke("terminal_contexts"))[id]?.titleCli;
      return process?.cli === "copilot";
    });
    const unsupported = await invoke("inspect_cli_usage", {
      targets: [{ id, process }],
      force: false,
    });
    if (unsupported.entries[0]?.status !== "unsupported")
      throw Error(
        "A provider-dependent agent did not report unavailable quota",
      );
    offlineProviders.push({
      cli: "copilot",
      status: unsupported.entries[0].status,
      numericWindows: unsupported.entries[0].windows.length,
    });
    await invoke("write_terminal", { id, data: "\u0003" });
    await wait(async () => !(await invoke("terminal_contexts"))[id]?.titleCli);
    await waitPrompt();
    await invoke("write_terminal", {
      id,
      data: "printf 'USAGE_NATIVE_OK\\n'\r",
    });
    await wait(() =>
      Array.from({ length: runtime.terminal.buffer.active.length }, (_, i) =>
        runtime.terminal.buffer.active.getLine(i)?.translateToString(),
      )
        .join("\n")
        .includes("USAGE_NATIVE_OK"),
    );
    checkpoint = "Settings main-host compatibility";
    await invoke("open_settings", { page: "agent-control" });
    await pause(1000);
    await invoke("plugin_smoke_result", {
      stage: "usage-settings",
      data: {
        offlineSmoke: "passed",
        offlineProviders,
        liveProbe: liveMode
          ? {
              mode: liveMode,
              status: liveProviders.every(
                (provider) =>
                  provider.status === "ready" && provider.readNumericWindows,
              )
                ? "passed"
                : "incomplete",
              providers: liveProviders,
            }
          : null,
      },
    });
  } catch (error) {
    const liveOnlyFailure = checkpoint.startsWith("live account usage:");
    await invoke("plugin_smoke_result", {
      stage: "failed",
      data: {
        checkpoint,
        error: String(error),
        offlineProviders,
        ...(liveOnlyFailure
          ? {
              offlineSmoke: "passed",
              liveProbe: {
                mode: liveMode,
                status: "incomplete",
                providers: liveProviders,
              },
            }
          : {}),
      },
    });
  }
})();
