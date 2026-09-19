import { Check } from "../icons";

const steps = ["Prepare phone", "Start Android", "Connect screen"];
const descriptions = [
  "Preparing the phone to start Android.",
  "Android is starting. The first start can take a little longer.",
  "Android is ready. Connecting to its screen.",
];

export default function PhoneStartup({
  step,
  stopping,
  onCancel,
}: {
  step: 0 | 1 | 2;
  stopping: boolean;
  onCancel: () => void;
}) {
  return (
    <div className="android-startup">
      <span className="android-startup-spinner" aria-hidden="true" />
      <div role="status">
        <h2>{stopping ? "Stopping phone…" : "Starting phone…"}</h2>
        <p>
          {stopping
            ? "Saving phone data and closing Android safely."
            : descriptions[step]}
        </p>
      </div>
      {!stopping && (
        <ol className="android-startup-steps" aria-label="Phone startup">
          {steps.map((label, index) => (
            <li
              key={label}
              className={index < step ? "is-complete" : ""}
              aria-current={index === step ? "step" : undefined}
            >
              <span className="android-startup-step" aria-hidden="true">
                {index < step ? <Check size={12} /> : index + 1}
              </span>
              {label}
            </li>
          ))}
        </ol>
      )}
      <button className="button" disabled={stopping} onClick={onCancel}>
        {stopping ? "Stopping…" : "Cancel start"}
      </button>
    </div>
  );
}
