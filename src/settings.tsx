import React from "react";
import ReactDOM from "react-dom/client";
import SettingsApp from "./SettingsApp";
import "./styles.css";
// macOS 外观覆盖层，必须排在 styles.css 之后
import "./macos.css";

const root = document.getElementById("root");
if (root) {
  ReactDOM.createRoot(root).render(
    <React.StrictMode>
      <SettingsApp />
    </React.StrictMode>
  );
}
