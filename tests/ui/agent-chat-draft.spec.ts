import { expect, test } from "@playwright/test";
import { mockDesktop } from "./desktop";
import { mockChats } from "./chat-mock";

test.beforeEach(async ({ page }) => {
  await mockDesktop(page, false);
  await mockChats(page);
  await page.goto("/");
  await page.getByRole("button", { name: /^New tab/ }).click();
  await page.getByRole("menuitem", { name: "Chat AI", exact: true }).click();
  await expect(
    page.getByRole("textbox", { name: "Message", exact: true }),
  ).toBeVisible();
});

test("agent draft retains human typing during native commit and autosaves against its new revision", async ({
  page,
}) => {
  await page.evaluate(async () => {
    const desktop = window as any;
    const module = await import(/* @vite-ignore */ "/src/chat/chat-runtime.ts");
    const id = Object.keys(desktop.__chatTest.conversations)[0];
    const runtime = module.existing(id);
    await runtime.ready;
    desktop.__draftRuntime = runtime;
    desktop.__draftSdk = runtime.chat;
    desktop.__agentDraft = runtime.applyAgentDraft(
      "0",
      "0",
      "Agent draft",
      async () => {
        const draft = await module.main({
          action: "draft",
          id,
          expected: 0,
          text: "Agent draft",
        });
        desktop.__draftCommitted = true;
        await new Promise<void>((resolve) => {
          desktop.__releaseDraft = resolve;
        });
        return draft;
      },
    );
  });
  await expect
    .poll(() => page.evaluate(() => (window as any).__draftCommitted))
    .toBe(true);
  const input = page.getByRole("textbox", { name: "Message", exact: true });
  await input.fill("Human text 日本語 🙂");
  // The ordinary autosave must queue behind the delayed native acknowledgement.
  await page.waitForTimeout(400);
  const result = await page.evaluate(async () => {
    const d = window as any;
    d.__releaseDraft();
    const result = await d.__agentDraft;
    await d.__draftRuntime.flush();
    const module = await import(/* @vite-ignore */ "/src/chat/chat-runtime.ts");
    const runtime = module.existing(d.__draftRuntime.id);
    return {
      retained: result.humanTextRetained,
      sameRuntime:
        runtime === d.__draftRuntime && runtime.chat === d.__draftSdk,
      draft: d.__chatTest.conversations[runtime.id].draft,
      dirty: runtime.dirty,
      error: runtime.snapshot.error,
      starts: d.__chatTest.starts,
    };
  });
  expect(result).toEqual({
    retained: true,
    sameRuntime: true,
    draft: { text: "Human text 日本語 🙂", revision: 2, attachments: [] },
    dirty: false,
    error: "",
    starts: 0,
  });
  await expect(input).toHaveValue("Human text 日本語 🙂");
});

test("agent draft rejects dirty or stale input and preserves text after native CAS failure", async ({
  page,
}) => {
  const results = await page.evaluate(async () => {
    const d = window as any;
    const module = await import(/* @vite-ignore */ "/src/chat/chat-runtime.ts");
    const id = Object.keys(d.__chatTest.conversations)[0];
    const runtime = module.existing(id);
    await runtime.ready;
    let commits = 0;
    const commit = async () => {
      commits++;
      throw Error("REVISION_CONFLICT");
    };
    const attempt = (draft: string, conversation: string) =>
      runtime
        .applyAgentDraft(draft, conversation, "Agent", commit)
        .catch((e: Error) => e.message);
    runtime.setText("Unsaved human text");
    const dirty = await attempt("0", "0");
    await runtime.flush();
    const staleDraft = await attempt("0", "0");
    const staleConversation = await attempt("1", "99");
    const nativeConflict = await attempt("1", "0");
    return {
      dirty,
      staleDraft,
      staleConversation,
      nativeConflict,
      commits,
      text: runtime.snapshot.text,
      draft: runtime.snapshot.loaded.draft.text,
      starts: d.__chatTest.starts,
    };
  });
  expect(results).toEqual({
    dirty: "REVISION_CONFLICT",
    staleDraft: "REVISION_CONFLICT",
    staleConversation: "REVISION_CONFLICT",
    nativeConflict: "REVISION_CONFLICT",
    commits: 1,
    text: "Unsaved human text",
    draft: "Unsaved human text",
    starts: 0,
  });
});
