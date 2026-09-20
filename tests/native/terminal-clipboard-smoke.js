(async () => {
  const directory = SMOKE_DIRECTORY;
  const invoke = window.__TAURI_INTERNALS__.invoke;
  const pause = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
  let runtime;
  let checkpoint = "startup";
  const screen = () =>
    runtime
      ? Array.from({ length: runtime.terminal.buffer.active.length }, (_, i) =>
          runtime.terminal.buffer.active.getLine(i)?.translateToString(),
        ).join("\n")
      : "";
  const wait = async (check) => {
    for (let i = 0; i < 300; i++) {
      if (await check()) return;
      await pause(100);
    }
    throw Error(`Timed out: ${checkpoint}`);
  };
  const quote = (value) => "'" + value.replaceAll("'", "'\\''") + "'";
  try {
    const { runningTerminal } = await import("/src/terminal-runtime.ts");
    await wait(
      () =>
        (runtime = runningTerminal("clipboard-terminal"))?.getSnapshot()
          .status === "running",
    );
    for (const [index, stage] of [
      "clipboard-image",
      "clipboard-image-only",
    ].entries()) {
      checkpoint =
        index === 0
          ? "Cmd+V with image and text"
          : "native menu with image only";
      await invoke("plugin_smoke_result", { stage, data: null });
      runtime.execute(`python3 ${quote(`${directory}/capture.py`)} ${index}`);
      await wait(() => screen().includes(`CLIPBOARD_CAPTURE_READY_${index}`));
      runtime.terminal.focus();
      if (index === 0) {
        runtime.terminal.textarea.dispatchEvent(
          new KeyboardEvent("keydown", {
            key: "v",
            code: "KeyV",
            metaKey: true,
            bubbles: true,
            cancelable: true,
          }),
        );
      } else {
        await invoke("plugin_smoke_result", {
          stage: "clipboard-menu",
          data: null,
        });
      }
      await wait(() => screen().includes(`CLIPBOARD_CAPTURE_DONE_${index}`));
      const captured = await invoke("read_editor_file", {
        root: `${directory}/project with spaces`,
        relative: `captured-${index}.json`,
      });
      const data = JSON.parse(captured.content);
      if (
        !/^\x1b\[200~'.*\/terminal-clipboard\/image-[\w-]+\.png' \x1b\[201~$/.test(
          data,
        ) ||
        /[\r\n]/.test(data)
      ) {
        throw Error("PTY did not receive one bracketed image path");
      }
    }
    checkpoint = "native text fallback";
    await invoke("plugin_smoke_result", {
      stage: "clipboard-text",
      data: null,
    });
    if (
      (await invoke("paste_terminal_clipboard", { id: runtime.sessionId })) !==
      "zażółć 🦀\r\nsecond line"
    ) {
      throw Error("Text changed");
    }
    let denied = false;
    try {
      await invoke("paste_terminal_clipboard", { id: "missing-session" });
    } catch {
      denied = true;
    }
    if (!denied) throw Error("Unknown session read the clipboard");
    if (SMOKE_AGENTS) {
      await invoke("plugin_smoke_result", {
        stage: "clipboard-image-only",
        data: null,
      });
      checkpoint = "Codex attachment";
      runtime.execute(`codex --no-alt-screen -C ${quote(SMOKE_REPOSITORY)}`);
      await pause(5000);
      await runtime.pasteClipboard();
      await wait(() => screen().includes("[Image #1]"));
      await invoke("plugin_smoke_result", {
        stage: "clipboard-screenshot",
        data: "codex",
      });
      runtime.send("\x03");
      await pause(300);
      runtime.send("\x03");
      await pause(1500);
      checkpoint = "agy native image attachment";
      runtime.execute(`cd ${quote(SMOKE_REPOSITORY)} && agy`);
      await pause(5000);
      await runtime.pasteClipboard();
      await wait(() => screen().includes("1 media attached"));
      await invoke("plugin_smoke_result", {
        stage: "clipboard-screenshot",
        data: "agy",
      });
      runtime.send("\x03");
      await pause(300);
      runtime.send("\x04");
      await pause(500);
    }
    checkpoint = "settings isolation";
    await invoke("plugin_smoke_result", {
      stage: "clipboard-settings",
      data: runtime.sessionId,
    });
  } catch (error) {
    await invoke("plugin_smoke_result", {
      stage: "failed",
      data: { checkpoint, error: String(error), screen: screen() },
    });
  }
})();
