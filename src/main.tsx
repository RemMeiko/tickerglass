import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import "./styles.css";
// macOS 外观覆盖层，必须排在 styles.css 之后（靠加载顺序赢同特异性的规则）
import "./macos.css";

const root = document.getElementById("root");
if (root) {
  ReactDOM.createRoot(root).render(
    <React.StrictMode>
      <App />
    </React.StrictMode>
  );
}
