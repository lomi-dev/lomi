import { createOpenAI } from "@ai-sdk/openai";
import { createAnthropic } from "@ai-sdk/anthropic";
import { createGoogle } from "@ai-sdk/google";
import { createOpenAICompatible } from "@ai-sdk/openai-compatible";
import { providerPresets } from "../../../src/chat/provider-presets.ts";
import type { Generation } from "./protocol.ts";

export function modelFor(input: Generation) {
  switch (input.provider) {
    case "custom": {
      // Never inherit ambient credentials or follow redirects to another server.
      const customFetch: typeof fetch = (url, init) => {
        const headers = new Headers(init?.headers);
        if (!input.apiKey) {
          headers.delete("authorization");
          headers.delete("x-api-key");
        }
        return globalThis.fetch(url, { ...init, headers, redirect: "error" });
      };
      const options = {
        baseURL: input.baseUrl!.replace(/\/+$/, ""),
        apiKey: input.apiKey,
        fetch: customFetch,
      };
      switch (input.apiFormat) {
        case "responses":
          return createOpenAI(options).responses(input.model);
        case "anthropic-messages":
          return createAnthropic(options)(input.model);
        default:
          return createOpenAICompatible({
            ...options,
            name: "custom",
          }).chatModel(input.model);
      }
    }
    case "openai":
      return createOpenAI({ apiKey: input.apiKey }).responses(input.model);
    case "anthropic":
      return createAnthropic({ apiKey: input.apiKey })(input.model);
    case "google":
      return createGoogle({ apiKey: input.apiKey })(input.model);
    case "xai":
    case "openrouter":
    case "deepseek":
    case "nvidia":
      return createOpenAICompatible({
        name: input.provider,
        baseURL: providerPresets[input.provider].baseURL,
        apiKey: input.apiKey,
      }).chatModel(input.model);
  }
}
