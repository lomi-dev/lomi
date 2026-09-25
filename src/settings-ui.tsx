import type { ReactNode } from "react";

export function SettingsPage({
  title,
  description,
  status,
  actions,
  wide = false,
  busy,
  className = "",
  contentClassName = "",
  children,
}: {
  title: string;
  description?: ReactNode;
  status?: ReactNode;
  actions?: ReactNode;
  wide?: boolean;
  busy?: boolean;
  className?: string;
  contentClassName?: string;
  children?: ReactNode;
}) {
  return (
    <main
      className={`settings-page${wide ? " settings-page-wide" : ""}${className ? ` ${className}` : ""}`}
      aria-busy={busy}
    >
      <div
        className={`settings-page-content${contentClassName ? ` ${contentClassName}` : ""}`}
      >
        <header className="settings-page-heading">
          <div className="settings-page-title">
            <h1>{title}</h1>
            {description && <p>{description}</p>}
          </div>
          {(status !== undefined || actions) && (
            <div className="settings-page-actions">
              {status !== undefined && (
                <span className="settings-status" role="status">
                  {status}
                </span>
              )}
              {actions}
            </div>
          )}
        </header>
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
  className = "",
  children,
}: {
  title: string;
  count?: number;
  description?: ReactNode;
  actions?: ReactNode;
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
      {children}
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

export function SettingsNotice({
  tone = "info",
  role = tone === "error" ? "alert" : undefined,
  action,
  className = "",
  children,
}: {
  tone?: "info" | "warning" | "error";
  role?: "alert" | "status" | "note";
  action?: ReactNode;
  className?: string;
  children: ReactNode;
}) {
  return (
    <div
      className={`settings-notice${className ? ` ${className}` : ""}`}
      data-tone={tone}
      role={role}
    >
      <div className="settings-notice-body">{children}</div>
      {action}
    </div>
  );
}
