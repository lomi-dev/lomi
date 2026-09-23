import React, { useState } from "react";
import { createRoot } from "react-dom/client";

function Fixture() {
  const [name, setName] = useState("");
  const [message, setMessage] = useState("Ready");
  return (
    <main>
      <h1>Lomi native browser qualification</h1>
      <label>
        Name{" "}
        <input
          id="name"
          value={name}
          onChange={(event) => setName(event.target.value)}
          onKeyDown={(event) => {
            window.fixtureKeys = [
              ...(window.fixtureKeys || []),
              { key: event.key, trusted: event.nativeEvent.isTrusted },
            ];
          }}
        />
      </label>
      <button
        id="save"
        onClick={(event) => {
          window.fixtureSaveCount = (window.fixtureSaveCount || 0) + 1;
          window.fixtureTrusted = event.nativeEvent.isTrusted;
          setMessage(name ? `Saved: ${name}` : "Name is required");
        }}
      >
        Save
      </button>
      <p id="result" role="status">
        {message}
      </p>
      <label>
        Choice{" "}
        <select id="choice">
          <option value="a">Alpha</option>
          <option value="b">Beta</option>
        </select>
      </label>
      <div
        id="editable"
        aria-label="Editable field"
        contentEditable
        suppressContentEditableWarning
      >
        Editable fixture
      </div>
      <input id="secret" type="password" defaultValue="fixture-password" />
      <input type="hidden" value="fixture-hidden" />
      <button id="spa" onClick={() => history.pushState({}, "", "#saved")}>
        SPA navigation
      </button>
      <canvas id="canvas" width="160" height="80" />
      <div aria-hidden="true" style={{ height: 2000 }} />
    </main>
  );
}
createRoot(document.getElementById("root")).render(<Fixture />);
window.fixtureReady = true;

if (window.fixtureAgentPermissions) {
  window.fixtureMedia = "pending";
  if (!navigator.mediaDevices) window.fixtureMedia = "unavailable";
  else
    navigator.mediaDevices.getUserMedia({ audio: true, video: true }).then(
      (stream) => {
        stream.getTracks().forEach((track) => track.stop());
        window.fixtureMedia = "granted";
      },
      (error) => {
        window.fixtureMedia = error.name;
      },
    );
}
