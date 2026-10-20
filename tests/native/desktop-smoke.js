(async () => {
  const { root, phase, idleMs } = SMOKE_CONFIG;
  const invoke = window.__TAURI_INTERNALS__.invoke;
  const checks = [];
  let checkpoint = "startup";
  const assert = (value, message) => {
    if (!value) throw Error(message);
  };
  const wait = async (test, message) => {
    for (let i = 0; i < 300; i++) {
      if (await test()) return;
      await new Promise((resolve) => setTimeout(resolve, 50));
    }
    throw Error(`Timed out: ${message}`);
  };
  const read = (relative) => invoke("read_editor_file", { root, relative });
  const save = (relative, file, content) =>
    invoke("save_editor_file", {
      request: { root, relative, revision: file.revision, content },
    });
  const operation = (relative, operation) =>
    invoke("file_operation", { root, relative, operation });
  const rejects = async (run, message) => {
    let denied = false;
    try {
      await run();
    } catch {
      denied = true;
    }
    assert(denied, message);
  };
  const treeButton = (name) =>
    [...document.querySelectorAll(".file-tree button")].find(
      (node) => node.textContent.trim() === name,
    );
  try {
    await wait(
      () =>
        document
          .querySelector(".cm-content")
          ?.textContent.includes("Desktop fixture"),
      "restored native editor",
    );
    checks.push(
      "packaged frontend restores file tab and renders real CodeMirror document",
    );
    if (phase === "exercise" && idleMs > 0) {
      checkpoint = "optional idle background and reopen";
      await invoke("plugin_smoke_result", {
        stage: "idle-visible",
        data: null,
      });
      await new Promise((resolve) => setTimeout(resolve, idleMs));
      await invoke("plugin_smoke_result", { stage: "idle-close", data: null });
      await wait(
        async () =>
          !(await invoke("plugin_smoke_result", {
            stage: "idle-state",
            data: null,
          })),
        "native close-to-background",
      );
      await invoke("plugin_smoke_result", { stage: "idle-hidden", data: null });
      await new Promise((resolve) => setTimeout(resolve, idleMs));
      await invoke("plugin_smoke_result", { stage: "idle-reopen", data: null });
      await wait(
        async () =>
          await invoke("plugin_smoke_result", {
            stage: "idle-state",
            data: null,
          }),
        "native background restore visibility",
      );
      await invoke("plugin_smoke_result", {
        stage: "idle-reopened",
        data: null,
      });
      assert(
        document
          .querySelector(".cm-content")
          ?.textContent.includes("Desktop fixture"),
        "Native close and reopen lost the editor document",
      );
      checks.push(
        navigator.platform.includes("Win")
          ? "native minimize and restore retains editor document"
          : "native close hides window and reopen retains editor document",
      );
    }
    if (phase === "restore") {
      checkpoint = "restart persistence";
      const prefs = await invoke("load_editor_preferences");
      assert(
        prefs.tabSize === 7 && prefs.insertSpaces === false,
        "Editor preferences did not survive restart",
      );
      const inbox = await invoke("load_notifications");
      assert(
        inbox.items.some(
          (item) => item.title === "Desktop native persistence" && item.read,
        ),
        "Notification read state did not survive restart",
      );
      const session = await invoke("load_session");
      assert(
        session.version === 5 && session.projects[0].path === root,
        "Session did not survive restart",
      );
      const history = await invoke("git_history", { root, skip: 0 });
      assert(
        history.commits[0].subject === "Desktop smoke local commit",
        "Native commit history did not survive restart",
      );
      checks.push(
        "session, editor settings, notification read state and native Git history survive process restart",
      );
    } else {
      checkpoint = "editor encodings";
      for (const [relative, encoding, original, replacement] of [
        [
          "utf8.txt",
          "utf8",
          "Zażółć 日本語 😀\r\nsecond\r\n",
          "Saved żółć 日本語 😀\r\nsecond\r\n",
        ],
        ["utf8-bom.txt", "utf8-bom", "BOM żółć\r\n", "Saved BOM żółć\r\n"],
        [
          "utf16le.txt",
          "utf16-le",
          "UTF16 żółć 😀\r\n",
          "Saved UTF16 żółć 😀\r\n",
        ],
        [
          "utf16be.txt",
          "utf16-be",
          "UTF16 żółć 😀\r\n",
          "Saved UTF16 żółć 😀\r\n",
        ],
      ]) {
        const file = await read(relative);
        assert(
          file.encoding === encoding && file.content === original,
          `Read differs: ${relative}: ${JSON.stringify(file)}`,
        );
        const revision = await save(relative, file, replacement);
        const saved = await read(relative);
        assert(
          saved.content === replacement &&
            saved.encoding === encoding &&
            saved.revision === revision,
          `Save differs: ${relative}`,
        );
      }
      checks.push(
        "native UTF-8, BOM, UTF-16 LE/BE reads and atomic saves preserve Unicode and CRLF",
      );
      checkpoint = "watch and conflict";
      const conflict = await read("conflict.txt");
      const events = [];
      const handler = window.__TAURI_INTERNALS__.transformCallback((event) =>
        events.push(event.payload),
      );
      const listener = await invoke("plugin:event|listen", {
        event: "editor-files-changed",
        target: { kind: "Any" },
        handler,
      });
      await invoke("watch_editor_files", {
        files: [{ root, relative: "conflict.txt" }],
      });
      await invoke("plugin_smoke_result", {
        stage: "external-write",
        data: null,
      });
      await wait(
        () =>
          events.some((paths) =>
            paths.some((path) => /conflict\.txt$/.test(path)),
          ),
        "real OS editor watcher event",
      );
      let conflictError;
      try {
        await save("conflict.txt", conflict, "must not overwrite");
      } catch (error) {
        conflictError = error;
      }
      assert(
        conflictError?.kind === "conflict",
        `Stale save was not rejected as conflict: ${JSON.stringify(conflictError)}`,
      );
      assert(
        (await read("conflict.txt")).content === "external change\r\n",
        "Conflict overwrote external bytes",
      );
      await invoke("plugin:event|unlisten", {
        event: "editor-files-changed",
        eventId: listener,
      });
      // Restore the frontend-owned watch set through subsequent editor opening.
      checks.push(
        "external filesystem write emits real native watcher event and stale save preserves external bytes",
      );
      checkpoint = "Explorer mutations";
      await operation("", { kind: "newFolder", name: "native-created" });
      await wait(
        () => treeButton("native-created"),
        "Explorer automatically sees native create",
      );
      await operation("native-created", { kind: "newFile", name: "empty.txt" });
      let file = await read("native-created/empty.txt");
      await save(
        "native-created/empty.txt",
        file,
        "Explorer copied bytes żółć\r\n",
      );
      await operation("native-created/empty.txt", {
        kind: "rename",
        name: "EMPTY.txt",
      });
      const entries = await invoke("list_directory", {
        root,
        relative: "native-created",
      });
      assert(
        entries.some((entry) => entry.name === "EMPTY.txt"),
        "Case-only rename did not change on-disk spelling",
      );
      await operation("", {
        kind: "copy",
        sourceRoot: root,
        source: "native-created/EMPTY.txt",
      });
      await rejects(
        () =>
          operation("", {
            kind: "copy",
            sourceRoot: root,
            source: "native-created/EMPTY.txt",
          }),
        "Explorer copy overwrote destination",
      );
      await rejects(
        () => operation("../", { kind: "newFile", name: "escape.txt" }),
        "Explorer accepted parent traversal",
      );
      for (const name of [".git", ".GIT", ".Git"]) {
        await rejects(
          () => operation("", { kind: "newFolder", name }),
          "Explorer accepted protected Git metadata name",
        );
        await rejects(
          () => operation(`${name}/config`, { kind: "delete" }),
          "Explorer accepted Git metadata traversal",
        );
      }
      await operation("native-created", { kind: "delete" });
      await wait(
        () => !treeButton("native-created"),
        "Explorer automatically sees native delete",
      );
      assert(
        (await read("EMPTY.txt")).content === "Explorer copied bytes żółć\r\n",
        "Explorer copy lost bytes",
      );
      checks.push(
        "Explorer live watcher create/delete, file create, case rename, copy, overwrite and containment protection",
      );
      checkpoint = "dirty editor Explorer relocation";
      await wait(
        () => treeButton("dirty-native.txt"),
        "new editor fixture in Explorer",
      );
      treeButton("dirty-native.txt").click();
      await wait(
        () => document.querySelector(".cm-content")?.textContent === "",
        "empty native editor",
      );
      let content = document.querySelector(".cm-content");
      content.focus();
      assert(
        document.execCommand("insertText", false, "Dirty native żółć 😀"),
        "Native CodeMirror insertText failed",
      );
      await wait(
        () =>
          document
            .querySelector(".cm-content")
            ?.textContent.includes("Dirty native żółć 😀"),
        "dirty CodeMirror text",
      );
      assert(
        (await read("dirty-native.txt")).content === "",
        "Dirty edit saved before rename",
      );
      treeButton("dirty-native.txt").focus();
      treeButton("dirty-native.txt").dispatchEvent(
        new KeyboardEvent("keydown", { key: "F2", code: "F2", bubbles: true }),
      );
      await wait(
        () => document.querySelector('[aria-label="Rename name"]'),
        "Explorer inline rename",
      );
      const rename = document.querySelector('[aria-label="Rename name"]');
      Object.getOwnPropertyDescriptor(
        HTMLInputElement.prototype,
        "value",
      ).set.call(rename, "renamed-native.txt");
      rename.dispatchEvent(new Event("input", { bubbles: true }));
      await new Promise((resolve) => requestAnimationFrame(resolve));
      const currentRename = document.querySelector(
        '[aria-label="Rename name"]',
      );
      assert(
        currentRename?.value === "renamed-native.txt",
        "Inline rename draft was lost before submit",
      );
      currentRename.form.requestSubmit();
      await wait(
        () =>
          treeButton("renamed-native.txt") && !treeButton("dirty-native.txt"),
        "Explorer relocated dirty document",
      );
      assert(
        document
          .querySelector(".cm-content")
          ?.textContent.includes("Dirty native żółć 😀"),
        "Rename discarded dirty text",
      );
      document.querySelector(".cm-content").focus();
      window.dispatchEvent(
        new KeyboardEvent("keydown", {
          key: "s",
          code: "KeyS",
          ctrlKey: !navigator.platform.includes("Mac"),
          metaKey: navigator.platform.includes("Mac"),
          bubbles: true,
        }),
      );
      await wait(
        async () =>
          (await read("renamed-native.txt")).content === "Dirty native żółć 😀",
        "save targets renamed native file",
      );
      const renamed = await read("renamed-native.txt");
      await save(
        "renamed-native.txt",
        renamed,
        "External renamed document żółć 😀",
      );
      await wait(
        () =>
          document
            .querySelector(".cm-content")
            ?.textContent.includes("External renamed document żółć 😀"),
        "relocated editor receives native watcher update",
      );
      document
        .querySelector('[data-tab-id="desktop-file"] [role="tab"]')
        .click();
      await wait(
        () =>
          document
            .querySelector(".cm-content")
            ?.textContent.includes("Desktop fixture"),
        "return to initial editor",
      );
      checks.push(
        "real CodeMirror dirty text survives Explorer inline rename; keyboard save and native watcher target relocated path",
      );
      checkpoint = "Git native commands";
      const gitFile = await read("tracked.txt");
      await save("tracked.txt", gitFile, "Native Git edited żółć\n");
      let status = await invoke("git_status", { root });
      assert(
        status.changes.some(
          (change) => change.path === "tracked.txt" && change.worktree === "M",
        ),
        "Git status missed working modification",
      );
      assert(
        (
          await invoke("git_diff", { root, path: "tracked.txt", staged: false })
        ).patch.includes("+Native Git edited żółć"),
        "Working diff lost contents",
      );
      await invoke("git_stage", { root, paths: ["tracked.txt"], stage: true });
      assert(
        (
          await invoke("git_diff", { root, path: "tracked.txt", staged: true })
        ).patch.includes("+Native Git edited żółć"),
        "Staged diff lost contents",
      );
      await invoke("git_stage", { root, paths: ["tracked.txt"], stage: false });
      status = await invoke("git_status", { root });
      assert(
        status.changes.some(
          (change) =>
            change.path === "tracked.txt" &&
            change.index === " " &&
            change.worktree === "M",
        ),
        "Unstage did not restore working modification",
      );
      await invoke("git_stage", { root, paths: ["tracked.txt"], stage: true });
      await invoke("git_commit", {
        root,
        message: "Desktop smoke local commit",
      });
      const history = await invoke("git_history", { root, skip: 0 });
      assert(
        history.commits.length === 2 &&
          history.commits[0].subject === "Desktop smoke local commit",
        "Commit history differs",
      );
      const details = await invoke("git_commit_details", {
        root,
        id: history.commits[0].id,
      });
      assert(
        details.authorEmail === "desktop-smoke@example.test" &&
          details.files.some((file) => file.path === "tracked.txt"),
        "Commit details or local author differs",
      );
      checks.push(
        "real local Git status, working/staged diff, stage/unstage, commit, history and commit details",
      );
      checkpoint = "settings and notifications";
      await invoke("save_editor_preferences", {
        data: { version: 1, tabSize: 7, insertSpaces: false },
      });
      await rejects(
        () =>
          invoke("save_editor_preferences", {
            data: { version: 1, tabSize: 0, insertSpaces: true },
          }),
        "Invalid editor preferences were accepted",
      );
      assert(
        (await invoke("load_editor_preferences")).tabSize === 7,
        "Invalid settings replaced persisted settings",
      );
      const inbox = await invoke("record_app_notification", {
        kind: "info",
        source: "app",
        title: "Desktop native persistence",
        body: "Disposable native desktop fixture",
      });
      const item = inbox.items.find(
        (item) => item.title === "Desktop native persistence",
      );
      assert(item && !item.read, "New notification was not persisted unread");
      await invoke("mark_notifications_read", { ids: [item.id] });
      document.querySelector('button[aria-label="Open account menu"]').click();
      await wait(
        () =>
          document.querySelector(
            '[role="menuitem"][aria-label="Notifications"]',
          ),
        "account menu",
      );
      document
        .querySelector('[role="menuitem"][aria-label="Notifications"]')
        .click();
      await wait(
        () =>
          document
            .querySelector(".notification-center")
            ?.textContent.includes("Desktop native persistence"),
        "native notification inbox",
      );
      document.querySelector('[aria-label="Close notifications"]').click();
      checks.push(
        "validated editor settings and real account-menu notification inbox with persisted read state",
      );
      checkpoint = "session save";
      const session = await invoke("load_session");
      await invoke("save_session", { data: session });
      assert(
        (await invoke("load_session")).projects[0].path === root,
        "Session native roundtrip differs",
      );
      checks.push(
        "native version-5 session save/load preserves disposable project",
      );
    }
    await invoke("plugin_smoke_result", {
      stage: "passed",
      data: { phase, checks },
    });
  } catch (error) {
    await invoke("plugin_smoke_result", {
      stage: "failed",
      data: {
        phase,
        checkpoint,
        error: String(error),
        detail: error,
        checks,
        renameInputs: [...document.querySelectorAll(".tree-create input")].map(
          (input) => ({
            label: input.getAttribute("aria-label"),
            value: input.value,
            readOnly: input.readOnly,
            connected: input.isConnected,
          }),
        ),
        dom: document.body.innerText.slice(0, 6000),
      },
    });
  }
})();
