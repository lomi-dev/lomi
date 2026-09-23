import { createRoot } from "react-dom/client";
import { Modal } from "../../src/ui";

export function mountProtectedModal() {
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  const close = () => {
    root.unmount();
    host.remove();
  };
  root.render(
    <Modal protectTheme title="Protected decision" onClose={close}>
      <p>The package must not hide this decision.</p>
      <button type="button" onClick={close}>
        Reject fixture action
      </button>
    </Modal>,
  );
}
