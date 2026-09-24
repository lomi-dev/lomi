import { expect, test } from "@playwright/test";
import { mockDesktop } from "./desktop";
import { mockChats } from "./chat-mock";

test.beforeEach(async ({ page }) => {
  await mockDesktop(page, false);
  await mockChats(page);
  await page.addInitScript(() => {
    const d = window as any;
    const invoke = d.__TAURI_INTERNALS__.invoke;
    d.__sendTest = {
      projection: null,
      pending: false,
      decisions: [],
      acks: [],
      calls: [],
    };
    d.__TAURI_INTERNALS__.invoke = async (command: string, args: any) => {
      const s = d.__sendTest;
      if (command === "agent_control_ui_register") return "send-epoch";
      if (command === "agent_control_ui_publish") {
        s.projection = args.projection;
        return;
      }
      if (command === "agent_control_ui_claim") return;
      if (command === "agent_control_chat_send_prepare") return s.plan;
      if (command === "agent_control_chat_send_pending") return s.pending;
      if (command === "agent_control_chat_send_decide") {
        s.decisions.push(args);
        s.pending = false;
        return;
      }
      if (command === "agent_control_chat_send") {
        s.calls.push({ ...args, channel: undefined });
        if (s.fail) throw Error("REVISION_CONFLICT");
        const accepted = await d.__chatInvoke("chat_generate", {
          input: s.input,
          channel: args.channel,
        });
        return {
          accepted,
          result: {
            ...s.result,
            draftRevision: String(accepted.draftRevision),
          },
        };
      }
      if (command === "agent_control_ui_ack") {
        s.acks.push(args.ack);
        return;
      }
      return invoke(command, args);
    };
  });
  await page.goto("/");
  await page.getByRole("button", { name: /^New tab/ }).click();
  await page.getByRole("menuitem", { name: "Chat AI", exact: true }).click();
  await expect(
    page.getByRole("textbox", { name: "Message", exact: true }),
  ).toBeVisible();
  await page.evaluate(async () => {
    const d = window as any;
    const module = await import(/* @vite-ignore */ "/src/chat/chat-runtime.ts");
    const id = Object.keys(d.__chatTest.conversations)[0];
    const runtime = module.existing(id);
    await runtime.ready;
    runtime.setText("Approved message 日本語 🙂");
    await runtime.flush();
    d.__sendTest.runtime = runtime;
    d.__sendTest.sdk = runtime.chat;
  });
});

async function request(page: import("@playwright/test").Page, id: string) {
  await page.evaluate(async (operationId) => {
    const d = window as any,
      s = d.__sendTest;
    const runtime = s.runtime,
      loaded = runtime.snapshot.loaded;
    const p = s.projection,
      panel = p.panels.find((p: any) => p.kind === "chat");
    s.pending = true;
    s.plan = {
      planHash: "a".repeat(64),
      conversationTitle: "Chat AI",
      connectionId: "fixture",
      connectionName: "Fixture provider",
      provider: "openai",
      model: "gpt-4.1",
      maxOutputTokens: 4096,
      temperature: null,
      draftText: runtime.snapshot.text,
      system: "Private system instructions",
      messageCount: 1,
      contextBytes: 99,
      attachments: [
        { name: "Résumé 日本語.txt", mime: "text/plain", byteLength: 24 },
      ],
    };
    s.input = {
      requestId: `${operationId}-request`,
      userId: `${operationId}-user`,
      assistantId: `${operationId}-assistant`,
      conversationId: runtime.id,
      expectedRevision: loaded.conversation.revision,
      draftRevision: loaded.draft.revision,
      action: "send",
      targetId: null,
      text: runtime.snapshot.text,
    };
    s.result = {
      workspaceId: panel.workspaceId,
      panelId: panel.id,
      conversationId: runtime.id,
      requestId: s.input.requestId,
      userId: s.input.userId,
      assistantId: s.input.assistantId,
      connectionId: "fixture",
      model: "gpt-4.1",
      rejection: null,
    };
    await d.__nativeTest.emitEvent("agent-control-command", {
      operationId,
      nonce: `${operationId}-nonce`,
      uiEpoch: "send-epoch",
      domainRevision: p.revision,
      projectId: p.workspaces.find((w: any) => w.id === panel.workspaceId)
        .projectId,
      action: {
        type: "send_chat",
        workspaceId: panel.workspaceId,
        input: {
          workspaceId: panel.workspaceId,
          panelId: panel.id,
          conversationId: runtime.id,
          connectionId: "fixture",
          model: "gpt-4.1",
          expectedDraftRevision: String(loaded.draft.revision),
          expectedConversationRevision: String(loaded.conversation.revision),
          expectedRevision: p.revision,
          retryEpoch: "epoch",
          requestKey: operationId,
        },
        requestId: s.input.requestId,
        userId: s.input.userId,
        assistantId: s.input.assistantId,
        notAfterMillis: String(Date.now() + 120000),
      },
    });
  }, id);
}

test("send approval defaults to cancel, exposes exact context and expires on human edits", async ({
  page,
}) => {
  await page.setViewportSize({ width: 900, height: 600 });
  await request(page, "declined");
  const dialog = page.getByRole("dialog", {
    name: "Send this message for the agent?",
  });
  await expect(dialog).toBeVisible();
  await expect(
    dialog.getByRole("button", { name: "Cancel", exact: true }),
  ).toBeFocused();
  await expect(dialog).toContainText("Fixture provider (fixture)");
  await expect(dialog).toContainText("gpt-4.1");
  await expect(dialog).toContainText("Résumé 日本語.txt");
  await expect(
    dialog.getByRole("textbox", { name: "Message to send" }),
  ).toHaveValue("Approved message 日本語 🙂");
  await dialog
    .getByText("System instructions included", { exact: true })
    .click();
  await expect(
    dialog.getByRole("textbox", { name: "System instructions", exact: true }),
  ).toHaveValue("Private system instructions");
  await dialog
    .getByRole("button", { name: "Send message", exact: true })
    .scrollIntoViewIfNeeded();
  await page.screenshot({ path: "test-results/mcp-chat-send-approval.png" });
  await page.keyboard.press("Escape");
  await expect(dialog).toHaveCount(0);
  expect(await page.evaluate(() => (window as any).__chatTest.starts)).toBe(0);
  await request(page, "edited");
  await expect(dialog).toBeVisible();
  await page.evaluate(() =>
    (window as any).__sendTest.runtime.setText("New human text"),
  );
  await expect(dialog).toHaveCount(0);
  await expect(
    page.getByRole("textbox", { name: "Message", exact: true }),
  ).toHaveValue("New human text");
  expect(await page.evaluate(() => (window as any).__chatTest.starts)).toBe(0);
});

test("saved YOLO mode sends only after exact native approval", async ({
  page,
}) => {
  await page.evaluate(() => {
    const desktop = window as any;
    desktop.__nativeTest.agentControlStartup = {
      ...desktop.__nativeTest.agentControlStartup,
      supported: true,
      yoloMode: true,
    };
  });
  const modeReadsBefore = await page.evaluate(
    () =>
      (window as any).__nativeTest.calls.filter(
        (call: any) => call.command === "agent_control_startup_state",
      ).length,
  );
  await request(page, "yolo-send");

  await expect(
    page.getByRole("dialog", { name: "Send this message for the agent?" }),
  ).toHaveCount(0);
  await expect
    .poll(() => page.evaluate(() => (window as any).__sendTest.acks.length))
    .toBe(1);
  const result = await page.evaluate(() => {
    const desktop = window as any;
    return {
      decisions: desktop.__sendTest.decisions,
      sends: desktop.__sendTest.calls,
      starts: desktop.__chatTest.starts,
      ack: desktop.__sendTest.acks[0],
      modeReads: desktop.__nativeTest.calls.filter(
        (call: any) => call.command === "agent_control_startup_state",
      ).length,
    };
  });
  expect(result.modeReads).toBe(modeReadsBefore + 1);
  expect(result.decisions).toEqual([
    {
      operationId: "yolo-send",
      nonce: "yolo-send-nonce",
      planHash: "a".repeat(64),
      approved: true,
    },
  ]);
  expect(result.sends).toHaveLength(1);
  expect(result.starts).toBe(1);
  expect(result.ack.result.kind).toBe("chat_sent");
});

test("an unavailable YOLO setting falls back to the ordinary send approval", async ({
  page,
}) => {
  await page.evaluate(() => {
    const desktop = window as any;
    desktop.__nativeTest.agentControlStartup = {
      ...desktop.__nativeTest.agentControlStartup,
      supported: true,
      yoloMode: true,
    };
    const invoke = desktop.__TAURI_INTERNALS__.invoke;
    desktop.__TAURI_INTERNALS__.invoke = (command: string, args: any) =>
      command === "agent_control_startup_state"
        ? Promise.reject(new Error("settings unavailable"))
        : invoke(command, args);
  });
  await request(page, "yolo-read-error");

  await expect(
    page.getByRole("dialog", { name: "Send this message for the agent?" }),
  ).toBeVisible();
  expect(await page.evaluate(() => (window as any).__sendTest.calls)).toEqual(
    [],
  );
  expect(
    await page.evaluate(() => (window as any).__sendTest.decisions),
  ).toEqual([]);
});

test("approved send uses reserved identity and retains SDK and human next draft during delayed ACK", async ({
  page,
}) => {
  await page.evaluate(() => {
    (window as any).__chatTest.slowAck = true;
  });
  await request(page, "approved");
  await page
    .getByRole("dialog")
    .getByRole("button", { name: "Send message", exact: true })
    .click();
  await expect
    .poll(() => page.evaluate(() => (window as any).__chatTest.starts))
    .toBe(1);
  await page
    .getByRole("textbox", { name: "Message", exact: true })
    .fill("Human next draft 🙂");
  await expect
    .poll(() => page.evaluate(() => (window as any).__sendTest.acks.length))
    .toBe(1);
  const result = await page.evaluate(async () => {
    const d = window as any,
      s = d.__sendTest;
    await s.runtime.flush();
    return {
      sameSdk: s.runtime.chat === s.sdk,
      draft: d.__chatTest.conversations[s.runtime.id].draft.text,
      request: d.__chatTest.conversations[s.runtime.id].request.id,
      ack: s.acks[0],
      calls: s.calls,
      starts: d.__chatTest.starts,
    };
  });
  expect(result.sameSdk).toBe(true);
  expect(result.draft).toBe("Human next draft 🙂");
  expect(result.request).toBe("approved-request");
  expect(result.starts).toBe(1);
  expect(result.ack.result.kind).toBe("chat_sent");
  expect(result.calls).toHaveLength(1);
  expect(result.calls[0]).not.toHaveProperty("input");
});

test("native refusal restores the saved draft without a provider request", async ({
  page,
}) => {
  await page.evaluate(() => {
    (window as any).__sendTest.fail = true;
  });
  await request(page, "stale-native");
  await page
    .getByRole("dialog")
    .getByRole("button", { name: "Send message", exact: true })
    .click();
  await expect
    .poll(() => page.evaluate(() => (window as any).__sendTest.acks.length))
    .toBe(1);
  await expect(
    page.getByRole("textbox", { name: "Message", exact: true }),
  ).toHaveValue("Approved message 日本語 🙂");
  expect(await page.evaluate(() => (window as any).__chatTest.starts)).toBe(0);
  expect(
    await page.evaluate(() => (window as any).__sendTest.acks[0].result),
  ).toEqual({ kind: "failure", code: "REVISION_CONFLICT" });
});
