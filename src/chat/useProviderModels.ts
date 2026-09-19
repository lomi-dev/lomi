import { useEffect, useMemo, useState } from "react";
import { api, errorMessage } from "../api";
import { validModelId, type Provider } from "./provider-presets";

const catalogErrors: Record<string, string> = {
  auth: "This API key was not accepted. Check the key and try again.",
  permission: "This API key does not have permission to list models.",
  "rate-limit":
    "The provider is receiving too many requests. Try again shortly.",
  timeout: "Loading models timed out. Try again.",
  network: "Could not reach the provider. Check your connection and try again.",
};

export function useProviderModels(provider: Provider | undefined, key: string) {
  const [attempt, setAttempt] = useState(0);
  const apiKey = key.trim();
  const query = useMemo(() => Symbol(), [provider, apiKey, attempt]);
  const [result, setResult] = useState<{
    query: typeof query;
    status: "ready" | "error";
    models: string[];
    error: string;
  }>();

  useEffect(() => {
    if (!provider || !apiKey) return;
    let active = true;
    const timer = window.setTimeout(() => {
      void api<{
        status: string;
        models?: string[];
        result?: { code?: string };
      }>("chat_preview_models", {
        provider,
        apiKey,
      })
        .then((response) => {
          if (!active) return;
          if (response.status !== "completed")
            throw new Error(
              catalogErrors[response.result?.code ?? ""] ??
                "Could not load models from the provider. Try again.",
            );
          const models = [
            ...new Set(response.models?.filter(validModelId) ?? []),
          ];
          if (!models.length)
            throw new Error("No chat models are available for this API key.");
          setResult({ query, status: "ready", models, error: "" });
        })
        .catch((error) => {
          if (active)
            setResult({
              query,
              status: "error",
              models: [],
              error: errorMessage(error),
            });
        });
    }, 500);
    return () => {
      active = false;
      window.clearTimeout(timer);
    };
  }, [query, provider, apiKey]);

  const current = result?.query === query ? result : undefined;
  return {
    status: !provider || !apiKey ? "idle" : (current?.status ?? "loading"),
    models: current?.models ?? [],
    error: current?.error ?? "",
    retry: () => setAttempt((value) => value + 1),
  };
}
