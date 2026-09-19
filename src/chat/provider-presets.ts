export const providerPresets = {
  openai: {
    name: "OpenAI",
    baseURL: "https://api.openai.com/v1",
    format: "OpenAI Responses",
    keyURL: "https://platform.openai.com/api-keys",
    models: ["gpt-4.1", "gpt-4.1-2025-04-14"],
  },
  google: {
    name: "Google (AI Studio)",
    baseURL: "https://generativelanguage.googleapis.com/v1beta",
    format: "Google Generative Language",
    keyURL: "https://aistudio.google.com/apikey",
    models: ["gemini-2.5-flash", "gemini-2.5-pro"],
  },
  xai: {
    name: "xAI",
    baseURL: "https://api.x.ai/v1",
    format: "OpenAI-compatible Chat Completions",
    keyURL: "https://console.x.ai/",
    models: ["grok-4.6"],
  },
  openrouter: {
    name: "OpenRouter",
    baseURL: "https://openrouter.ai/api/v1",
    format: "OpenAI-compatible Chat Completions",
    keyURL: "https://openrouter.ai/settings/keys",
    models: ["openai/gpt-4.1", "google/gemini-2.5-flash"],
  },
  deepseek: {
    name: "DeepSeek",
    baseURL: "https://api.deepseek.com",
    format: "OpenAI-compatible Chat Completions",
    keyURL: "https://platform.deepseek.com/api_keys",
    models: ["deepseek-flash", "deepseek-v4-pro"],
  },
  nvidia: {
    name: "NVIDIA Build",
    baseURL: "https://integrate.api.nvidia.com/v1",
    format: "OpenAI-compatible Chat Completions",
    keyURL: "https://build.nvidia.com/",
    models: ["meta/llama-3.1-8b-instruct"],
  },
  anthropic: {
    name: "Anthropic",
    baseURL: "https://api.anthropic.com/v1",
    format: "Anthropic Messages",
    keyURL: "https://platform.claude.com/settings/keys",
    models: ["claude-opus-5"],
  },
} as const;

export type Provider = keyof typeof providerPresets;
export const providerIds = Object.keys(providerPresets) as Provider[];
export const validModelId = (value: string) =>
  /^[a-zA-Z0-9][a-zA-Z0-9._:/-]{0,199}$/.test(value);
