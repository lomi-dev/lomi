import { SettingsPage } from "../settings-ui";
import NotificationSettings from "./NotificationSettings";
import { useAuthState } from "./useAuthState";

export default function NotificationSettingsPage() {
  const { state } = useAuthState();
  return (
    <SettingsPage title="Notifications">
      <NotificationSettings
        key={`${state?.user?.id ?? "signed-out"}:${state?.session?.id ?? "none"}:${state?.status ?? "checking"}`}
        auth={state}
      />
    </SettingsPage>
  );
}
