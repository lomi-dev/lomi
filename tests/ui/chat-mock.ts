import type { Page } from "@playwright/test";
export async function mockChats(page: Page) {
  await page.addInitScript(() => {
    const desktop = window as any;
    const config = {
      connectionId: "fixture",
      model: "gpt-4.1",
      system: "",
      maxOutputTokens: 4096,
      temperature: null,
      configured: true,
    };
    let preferences = JSON.parse(
      localStorage.getItem("chat-preferences") ?? "null",
    ) ?? {
      version: 1,
      revision: 0,
      connections: [
        {
          id: "fixture",
          name: "Fixture",
          provider: "openai",
          enabled: true,
          credentialRevision: 1,
          secretMode: "session",
          secretId: "fixture-1",
          models: ["gpt-4.1"],
          testedModel: null,
          testStatus: null,
        },
      ],
      defaults: config,
      sendMode: "enter",
    };
    const conversations = JSON.parse(
      localStorage.getItem("chat-conversations") ?? "{}",
    );
    const requests = new Map<string, any>();
    const save = () =>
      localStorage.setItem("chat-conversations", JSON.stringify(conversations));
    desktop.__chatTest = {
      hold: false,
      maxChunks: Number.POSITIVE_INFINITY,
      slowAck: false,
      failClose: false,
      failDraft: false,
      failHistoryAction: "",
      pendingLists: [],
      response: "Zażółć 日本語 👩🏽‍💻 ",
      chunks: 5,
      connectionActions: [],
      modelPreviews: [],
      pendingModelPreviews: [],
      failPreferences: false,
      starts: 0,
      stops: 0,
      requests,
      conversations,
    };
    const stop = (request: any, status = "cancelled") => {
      if (request.done) return;
      request.done = true;
      clearInterval(request.timer);
      const loaded = conversations[request.input.conversationId];
      loaded.request.status = status;
      request.message.status = status;
      request.channel.onmessage({
        type: "chunk",
        epoch: request.epoch,
        sequence: ++request.sequence,
        chunk: { type: "text-end", id: "text" },
      });
      request.channel.onmessage({ type: "terminal", status });
      save();
    };
    desktop.__chatInvoke = async (command: string, args: any) => {
      if (command === "chat_preferences") return structuredClone(preferences);
      if (command === "chat_preview_models") {
        const models: Record<string, string[]> = {
          openai: ["gpt-4.1", "gpt-4.1-2025-04-14"],
          anthropic: ["claude-opus-5"],
          google: ["gemini-2.5-flash", "gemini-2.5-pro"],
          xai: ["grok-4.6"],
          openrouter: ["openai/gpt-4.1"],
          deepseek: ["deepseek-flash"],
          nvidia: ["meta/llama-3.1-8b-instruct"],
        };
        desktop.__chatTest.modelPreviews.push({ provider: args.provider });
        if (desktop.__chatTest.holdModelPreviews)
          return new Promise((resolve) => {
            desktop.__chatTest.pendingModelPreviews.push(resolve);
          });
        if (desktop.__chatTest.failModelPreview)
          return {
            status: "failed",
            result: { code: desktop.__chatTest.failModelPreview },
          };
        return {
          status: "completed",
          models: desktop.__chatTest.previewModels ?? models[args.provider],
        };
      }
      if (command === "chat_preferences_save") {
        if (desktop.__chatTest.failPreferences)
          throw "System credential store is locked.";
        if (args.expected !== preferences.revision)
          throw "conflict: Chat AI settings changed.";
        preferences = { ...args.data, revision: preferences.revision + 1 };
        if (args.newKey) {
          const c = preferences.connections.find(
            (c: any) => c.id === args.keyConnection,
          );
          c.credentialRevision++;
          c.secretId = `${c.id}-${c.credentialRevision}`;
        }
        if (args.clearKey) {
          const c = preferences.connections.find(
            (c: any) => c.id === args.clearKey,
          );
          c.secretId = null;
          c.credentialRevision++;
        }
        localStorage.setItem("chat-preferences", JSON.stringify(preferences));
        return structuredClone(preferences);
      }
      if (command === "chat_connection_action") {
        desktop.__chatTest.connectionActions.push(args);
        if (desktop.__chatTest.failConnectionAction)
          return { status: "failed", result: { code: "auth" } };
        const connection = preferences.connections.find(
          (c: any) => c.id === args.connectionId,
        );
        if (args.operation === "list-models") {
          connection.models = [
            ...new Set([
              ...connection.models,
              ...Array.from(
                { length: 12 },
                (_, index) => `catalog-model-${index}`,
              ),
            ]),
          ];
        } else {
          connection.testedModel = args.model;
          connection.testStatus = "completed";
        }
        preferences.revision++;
        localStorage.setItem("chat-preferences", JSON.stringify(preferences));
        return { status: "completed" };
      }
      if (
        command === "chat_retain" ||
        command === "chat_ack" ||
        command === "chat_flush" ||
        command === "chat_export"
      )
        return;
      if (command === "chat_close") {
        if (desktop.__chatTest.failClose) throw "storage: fixture failure";
        for (const request of requests.values())
          if (args.conversations.includes(request.input.conversationId))
            stop(request);
        return;
      }
      if (command === "chat_cancel") {
        desktop.__chatTest.stops++;
        const request = requests.get(args.requestId);
        if (request) stop(request);
        return;
      }
      if (command === "chat_subscribe") {
        const request = requests.get(args.requestId);
        if (!request) return false;
        request.channel = args.channel;
        request.epoch++;
        args.channel.onmessage({
          type: "snapshot",
          epoch: request.epoch,
          sequence: request.sequence,
          snapshot: {
            message: structuredClone(request.message),
            blocks: { text: { index: 0, type: "text", open: !request.done } },
          },
          terminal: request.done
            ? { type: "terminal", status: request.message.status }
            : null,
        });
        return true;
      }
      if (command === "chat_generate") {
        const input = args.input,
          loaded = conversations[input.conversationId];
        desktop.__chatTest.starts++;
        const user = {
          id: input.userId,
          role: "user",
          parts: [{ type: "text", text: input.text }],
          partsVersion: 1,
          status: "completed",
          attachments: [],
          metadata: {},
        };
        const assistant = {
          id: input.assistantId,
          parentId: user.id,
          role: "assistant",
          parts: [{ type: "text", text: "", state: "streaming" }],
          partsVersion: 1,
          status: "active",
          attachments: [],
          metadata: { model: config.model },
        };
        loaded.messages.push(user, assistant);
        loaded.request = {
          id: input.requestId,
          assistantId: input.assistantId,
          status: "active",
        };
        loaded.conversation.revision++;
        loaded.conversation.title = input.text.slice(0, 40);
        loaded.draft = {
          text: "",
          revision: loaded.draft.revision + 1,
          attachments: [],
        };
        save();
        const request = {
          input,
          channel: args.channel,
          sequence: 0,
          epoch: 0,
          message: assistant,
          done: false,
          timer: 0,
        };
        requests.set(input.requestId, request);
        setTimeout(() => {
          if (request.done) return;
          args.channel.onmessage({
            type: "chunk",
            epoch: 0,
            sequence: ++request.sequence,
            chunk: { type: "start", messageId: input.assistantId },
          });
          args.channel.onmessage({
            type: "chunk",
            epoch: 0,
            sequence: ++request.sequence,
            chunk: { type: "text-start", id: "text" },
          });
          let count = 0;
          request.timer = window.setInterval(() => {
            if (request.done || count >= desktop.__chatTest.maxChunks) return;
            const delta = desktop.__chatTest.response;
            assistant.parts[0].text += delta;
            request.channel.onmessage({
              type: "chunk",
              epoch: request.epoch,
              sequence: ++request.sequence,
              chunk: { type: "text-delta", id: "text", delta },
            });
            if (
              ++count === desktop.__chatTest.chunks &&
              !desktop.__chatTest.hold
            )
              stop(request, "completed");
          }, 60);
        }, 20);
        if (desktop.__chatTest.slowAck)
          await new Promise((r) => setTimeout(r, 700));
        return {
          requestId: input.requestId,
          assistantId: input.assistantId,
          userId: input.userId,
          draftRevision: loaded.draft.revision,
          repeated: false,
        };
      }
      if (command === "chat_main") {
        const input = args.input,
          loaded = conversations[input.id];
        if (input.action === "create") {
          const conversation = {
            id: input.id,
            title: "Chat AI",
            origin: input.origin,
            config: structuredClone(preferences.defaults),
            revision: 0,
            activeLeafId: null,
            updatedAt: Date.now(),
            pinned: false,
          };
          conversations[input.id] = {
            conversation,
            draft: { text: "", revision: 0, attachments: [] },
            messages: [],
            hasOlder: false,
            request: null,
          };
          save();
          return structuredClone(conversation);
        }
        if (input.action === "list") {
          const values = structuredClone(
            Object.values(conversations)
              .filter((v: any) =>
                (v.conversation.title + " " + JSON.stringify(v.messages))
                  .toLowerCase()
                  .includes(input.query.toLowerCase()),
              )
              .map((v: any) => v.conversation)
              .filter(
                (v: any) =>
                  (!input.workspace ||
                    v.origin.workspaceId === input.workspace) &&
                  (!input.project || v.origin.projectId === input.project),
              )
              .sort(
                (a: any, b: any) =>
                  Number(!!b.pinned) - Number(!!a.pinned) ||
                  b.updatedAt - a.updatedAt ||
                  a.id.localeCompare(b.id),
              )
              .slice(input.offset, input.offset + 50),
          );
          if (desktop.__chatTest.holdLists)
            return new Promise((resolve) =>
              desktop.__chatTest.pendingLists.push({
                query: input.query,
                resolve: () => resolve(values),
              }),
            );
          return values;
        }
        if (!loaded) throw "missing: This conversation is unavailable.";
        if (input.action === "load") return structuredClone(loaded);
        if (input.action === "draft") {
          if (desktop.__chatTest.failDraft)
            throw "storage: Draft fixture failure";
          if (loaded.draft.revision !== input.expected) throw "conflict: draft";
          loaded.draft = {
            ...loaded.draft,
            text: input.text,
            revision: loaded.draft.revision + 1,
          };
          save();
          return structuredClone(loaded.draft);
        }
        if (input.action === "configure") {
          loaded.conversation.config = input.config;
          loaded.conversation.revision++;
          save();
          return structuredClone(loaded.conversation);
        }
        if (["rename", "pin", "delete"].includes(input.action)) {
          if (desktop.__chatTest.failHistoryAction === input.action)
            throw "storage: History fixture failure";
          if (input.action === "rename") {
            loaded.conversation.title = input.title;
            loaded.conversation.updatedAt = Date.now();
          }
          if (input.action === "pin") loaded.conversation.pinned = input.pinned;
          if (input.action === "delete") {
            for (const request of requests.values())
              if (request.input.conversationId === input.id) stop(request);
            delete conversations[input.id];
            await desktop.__nativeTest.emitEvent(
              "chat-conversation-deleted",
              input.id,
            );
          }
          save();
          await desktop.__nativeTest.emitEvent("chat-history-changed");
          return input.action === "delete"
            ? null
            : structuredClone(loaded.conversation);
        }
      }
      throw Error(`Unexpected chat command: ${command}`);
    };
  });
}
