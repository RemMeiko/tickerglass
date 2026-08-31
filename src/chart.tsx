import React from "react";
import ReactDOM from "react-dom/client";
import ChartApp from "./ChartApp";
import "./styles.css";
import "./chart.css";
// macOS 外观覆盖层，必须排在 styles.css 与 chart.css 之后
import "./macos.css";

const root = document.getElementById("root");
if (root) {
  ReactDOM.createRoot(root).render(
    <React.StrictMode>
      <ChartApp />
    </React.StrictMode>
  );
}
