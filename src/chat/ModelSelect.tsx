import { useId, useState } from "react";
import Select from "../Select";
import type { Connection } from "./types";
import { modelLabel, modelOptions } from "./models";

export function ModelSelect({
  connection,
  value,
  onChange,
  availableModels,
  disabled = !connection,
  placeholder = "Choose a model",
  allowCustom = true,
}: {
  connection?: Pick<Connection, "provider" | "models">;
  value: string;
  onChange: (value: string) => void;
  availableModels?: string[];
  disabled?: boolean;
  placeholder?: string;
  allowCustom?: boolean;
}) {
  const modelId = useId();
  const options = availableModels ?? modelOptions(connection);
  const [custom, setCustom] = useState(false);
  const isCustom =
    allowCustom &&
    ((custom && !value) || (!!value && !options.includes(value)));
  return (
    <div className="chat-model-select">
      <div className="chat-select-field">
        <label htmlFor={modelId}>Model</label>
        <Select
          id={modelId}
          disabled={disabled}
          value={isCustom ? "custom" : value}
          onChange={(next) => {
            setCustom(next === "custom");
            onChange(next === "custom" ? "" : next);
          }}
          options={[
            { value: "", label: placeholder, disabled: true },
            ...options.map((id) => ({ value: id, label: modelLabel(id) })),
            ...(allowCustom
              ? [{ value: "custom", label: "Custom model…" }]
              : []),
          ]}
        />
      </div>
      {isCustom && (
        <label>
          Custom model ID
          <input
            required
            value={value}
            placeholder="Enter the provider’s model ID"
            spellCheck={false}
            onChange={(event) => onChange(event.target.value)}
          />
        </label>
      )}
    </div>
  );
}
