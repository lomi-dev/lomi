import { useId, useState } from "react";
import Select from "../Select";
import type { Connection } from "./types";
import { modelLabel, modelOptions } from "./models";

export function ModelSelect({
  connection,
  value,
  onChange,
  onCommit,
  availableModels,
  disabled = !connection,
  placeholder = "Choose a model",
  allowCustom = true,
  id,
  hideLabel = false,
}: {
  connection?: Pick<Connection, "provider" | "models">;
  value: string;
  onChange: (value: string) => void;
  /** Called when a listed model is picked or the custom ID field loses focus. */
  onCommit?: (value: string) => void;
  availableModels?: string[];
  disabled?: boolean;
  placeholder?: string;
  allowCustom?: boolean;
  id?: string;
  hideLabel?: boolean;
}) {
  const generatedId = useId();
  const modelId = id ?? generatedId;
  const options = availableModels ?? modelOptions(connection);
  const [custom, setCustom] = useState(false);
  const isCustom =
    allowCustom &&
    ((custom && !value) || (!!value && !options.includes(value)));
  const customInput = (
    <input
      required
      value={value}
      aria-label={hideLabel ? "Custom model ID" : undefined}
      placeholder="Enter the provider’s model ID"
      spellCheck={false}
      onChange={(event) => onChange(event.target.value)}
      onBlur={() => onCommit?.(value)}
      onKeyDown={(event) => {
        if (onCommit && event.key === "Enter") {
          event.preventDefault();
          onCommit(value);
        }
      }}
    />
  );
  return (
    <div className="chat-model-select">
      <div className="chat-select-field">
        {!hideLabel && <label htmlFor={modelId}>Model</label>}
        <Select
          id={modelId}
          disabled={disabled}
          value={isCustom ? "custom" : value}
          onChange={(next) => {
            setCustom(next === "custom");
            onChange(next === "custom" ? "" : next);
            if (next !== "custom") onCommit?.(next);
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
      {isCustom &&
        (hideLabel ? (
          customInput
        ) : (
          <label>
            Custom model ID
            {customInput}
          </label>
        ))}
    </div>
  );
}
