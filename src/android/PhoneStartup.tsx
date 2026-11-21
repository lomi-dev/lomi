import { Check, Smartphone } from "../icons";
import { msg, useT } from "../i18n";

const steps = [
  msg("Prepare phone"),
  msg("Start Android"),
  msg("Connect screen"),
];
const descriptions = [
  msg("Preparing the phone to start Android."),
  msg("Android is starting. The first start can take a little longer."),
  msg("Android is ready. Connecting to its screen."),
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
  const { t } = useT();
  return (
    <div className="android-panel-state android-startup">
      <header className="android-state-heading" role="status">
        <span className="android-state-icon" aria-hidden="true">
          <Smartphone size={22} />
          <span className="android-startup-spinner" />
        </span>
        <div className="android-state-copy">
          <h2>{stopping ? t("Stopping phone…") : t("Starting phone…")}</h2>
          <p>
            {stopping
              ? t("Saving phone data and closing Android safely.")
              : t(descriptions[step])}
          </p>
        </div>
      </header>
      {!stopping && (
        <ol className="android-startup-steps" aria-label={t("Phone startup")}>
          {steps.map((label, index) => (
            <li
              key={label}
              className={index < step ? "is-complete" : ""}
              aria-current={index === step ? "step" : undefined}
            >
              <span className="android-startup-step" aria-hidden="true">
                {index < step ? <Check size={12} /> : index + 1}
              </span>
              {t(label)}
            </li>
          ))}
        </ol>
      )}
      <div className="android-state-actions">
        <button className="button" disabled={stopping} onClick={onCancel}>
          {stopping ? t("Stopping…") : t("Cancel start")}
        </button>
      </div>
    </div>
  );
}
