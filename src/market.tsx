import React from "react";
import ReactDOM from "react-dom/client";
import MarketApp from "./MarketApp";
import "./styles.css";
import "./market.css";
// macOS 外观覆盖层，必须排在其他样式表之后
import "./macos.css";

const root = document.getElementById("root");
if (root) {
  ReactDOM.createRoot(root).render(
    <React.StrictMode>
      <MarketApp />
    </React.StrictMode>
  );
}
