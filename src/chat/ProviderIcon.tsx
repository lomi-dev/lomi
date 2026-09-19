import type { CSSProperties } from "react";
import type { Provider } from "./provider-presets";
import openai from "./provider-icons/openai.svg";
import anthropic from "./provider-icons/anthropic.svg";
import xai from "./provider-icons/xai.svg";
import openrouter from "./provider-icons/openrouter.svg";
import deepseek from "./provider-icons/deepseek.svg";
import nvidia from "./provider-icons/nvidia.svg";
import google from "./provider-icons/google.png";

const icons = { openai, anthropic, xai, openrouter, deepseek, nvidia };

export function ProviderIcon({ provider }: { provider: Provider }) {
  return (
    <span className="chat-provider-icon" aria-hidden="true">
      {provider === "google" ? (
        <img src={google} width={24} height={24} alt="" />
      ) : (
        <span
          className="chat-provider-logo"
          style={
            { "--provider-icon": `url("${icons[provider]}")` } as CSSProperties
          }
        />
      )}
    </span>
  );
}
