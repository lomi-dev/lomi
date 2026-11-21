import { useT } from "../i18n";
import { SettingsPage } from "../settings-ui";
import NotificationSettings from "./NotificationSettings";
import { useAuthState } from "./useAuthState";

export default function NotificationSettingsPage() {
  const { t } = useT();
  const { state } = useAuthState();
  return (
    <SettingsPage title={t("Notifications")}>
      <NotificationSettings
        key={`${state?.user?.id ?? "signed-out"}:${state?.session?.id ?? "none"}:${state?.status ?? "checking"}`}
        auth={state}
      />
    </SettingsPage>
  );
}
