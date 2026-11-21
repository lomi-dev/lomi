import { useId, type ReactNode } from "react";
import { Notice, type NoticeProps } from "./Notice";

export function SettingsPage({
  title,
  status,
  actions,
  busy,
  className = "",
  contentClassName = "",
  children,
}: {
  title: string;
  status?: ReactNode;
  actions?: ReactNode;
  busy?: boolean;
  className?: string;
  contentClassName?: string;
  children?: ReactNode;
}) {
  const titleId = useId();
  return (
    <main
      className={`settings-page${className ? ` ${className}` : ""}`}
      aria-busy={busy}
      aria-labelledby={titleId}
    >
      <div
        className={`settings-page-content${contentClassName ? ` ${contentClassName}` : ""}`}
      >
        <header className="settings-page-heading">
          <div className="settings-page-title">
            <h1 id={titleId}>{title}</h1>
          </div>
          {actions && <div className="settings-page-actions">{actions}</div>}
        </header>
        {/* Confirmations are transient toasts; progress text ("…") stays until it changes. */}
        <span className="settings-toast-region" role="status">
          {status ? (
            <span
              key={String(status)}
              className="settings-toast"
              data-progress={
                typeof status === "string" && status.endsWith("…")
                  ? true
                  : undefined
              }
            >
              {status}
            </span>
          ) : null}
        </span>
        {children}
      </div>
    </main>
  );
}

export function SettingsSection({
  title,
  count,
  description,
  actions,
  grouped = true,
  className = "",
  children,
}: {
  title: string;
  count?: number;
  description?: ReactNode;
  actions?: ReactNode;
  grouped?: boolean;
  className?: string;
  children: ReactNode;
}) {
  return (
    <section
      className={`settings-section${className ? ` ${className}` : ""}`}
      aria-label={title}
    >
      <header className="settings-section-heading">
        <div>
          <h2>
            {title}
            {count !== undefined && (
              <span className="settings-count">{count}</span>
            )}
          </h2>
          {description && <p>{description}</p>}
        </div>
        {actions && <div className="settings-section-actions">{actions}</div>}
      </header>
      <div
        className={`settings-section-body${grouped ? " settings-group" : ""}`}
      >
        {children}
      </div>
    </section>
  );
}

export function SettingRow({
  label,
  description,
  htmlFor,
  descriptionId,
  stacked = false,
  className = "",
  children,
}: {
  label: ReactNode;
  description?: ReactNode;
  htmlFor?: string;
  descriptionId?: string;
  stacked?: boolean;
  className?: string;
  children?: ReactNode;
}) {
  return (
    <div
      className={`setting-row${stacked ? " setting-row-stacked" : ""}${className ? ` ${className}` : ""}`}
    >
      <div className="setting-label">
        {htmlFor ? (
          <label htmlFor={htmlFor}>{label}</label>
        ) : (
          <span>{label}</span>
        )}
        {description && <small id={descriptionId}>{description}</small>}
      </div>
      {children && <div className="setting-controls">{children}</div>}
    </div>
  );
}

export function SettingsNotice({ className = "", ...props }: NoticeProps) {
  return (
    <Notice
      {...props}
      className={`settings-notice${className ? ` ${className}` : ""}`}
    />
  );
}
