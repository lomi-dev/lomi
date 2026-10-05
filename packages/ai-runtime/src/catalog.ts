import type { Generation, Emit } from "./protocol.ts";
import { errorCode } from "./protocol.ts";
import { providerPresets } from "../../../src/chat/provider-presets.ts";

export async function catalog(
  input: Generation,
  requestId: string,
  controller: AbortController,
  emit: Emit,
) {
  let sequence = 0;
  const send = (type: Parameters<Emit>[0]["type"], payload: unknown) =>
    emit({
      protocolVersion: 1,
      requestId,
      sequence: ++sequence,
      type,
      payload,
    });
  const timer = setTimeout(() => controller.abort(), 30_000);
  try {
    const anthropic =
      input.provider === "anthropic" ||
      (input.provider === "custom" && input.apiFormat === "anthropic-messages");
    const suffix =
      input.provider === "google"
        ? "?pageSize=1000"
        : anthropic
          ? "?limit=1000"
          : "";
    const baseUrl =
      input.provider === "custom"
        ? input.baseUrl!
        : providerPresets[input.provider].baseURL;
    const url = `${baseUrl.replace(/\/+$/, "")}/models${suffix}`;
    const headers: Record<string, string> = anthropic
      ? { "x-api-key": input.apiKey, "anthropic-version": "2023-06-01" }
      : input.provider === "google"
        ? { "x-goog-api-key": input.apiKey }
        : { Authorization: `Bearer ${input.apiKey}` };
    if (input.provider === "custom" && !input.apiKey) {
      delete headers.Authorization;
      delete headers["x-api-key"];
    }
    const response = await fetch(url, {
      headers,
      signal: controller.signal,
      redirect: "error",
    });
    if (!response.ok) throw { statusCode: response.status };
    if (!response.body) throw new Error("network");
    const chunks: Uint8Array[] = [];
    let bytes = 0;
    for await (const chunk of response.body) {
      if ((bytes += chunk.length) > 2 * 1024 * 1024) {
        controller.abort();
        throw new Error("catalog-limit");
      }
      chunks.push(chunk);
    }
    const data = JSON.parse(Buffer.concat(chunks).toString("utf8"));
    const values: unknown[] = data.data ?? data.models;
    if (!Array.isArray(values)) throw new Error("protocol");
    const models = values
      .flatMap((value) => {
        if (!value || typeof value !== "object") return [];
        const item = value as {
          id?: string;
          name?: string;
          supportedGenerationMethods?: string[];
          architecture?: { output_modalities?: string[] };
        };
        if (
          input.provider === "google" &&
          !item.supportedGenerationMethods?.includes("generateContent")
        )
          return [];
        if (
          input.provider === "openrouter" &&
          item.architecture?.output_modalities &&
          !item.architecture.output_modalities.includes("text")
        )
          return [];
        const modelId = item.id ?? item.name ?? "";
        const id =
          input.provider === "google"
            ? modelId.replace(/^models\//, "")
            : modelId;
        return /^[a-zA-Z0-9][a-zA-Z0-9._:/-]{0,199}$/.test(id) ? [id] : [];
      })
      .slice(0, 1000)
      .sort();
    await send("models", {
      models,
      fetchedAt: Date.now(),
      truncated: values.length >= 1000,
    });
    await send("completed", {});
  } catch (error) {
    await send(controller.signal.aborted ? "cancelled" : "failed", {
      code: controller.signal.aborted ? "timeout" : errorCode(error),
    });
  } finally {
    clearTimeout(timer);
  }
}
