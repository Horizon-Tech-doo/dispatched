import "./ui/styles.css";
import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { applyTheme, resolveTheme } from "./ui";

// Apply the theme before the first render so the first frame already has the variables.
applyTheme(resolveTheme("system", window.matchMedia("(prefers-color-scheme: dark)").matches), "compact");

const boot =
  import.meta.env.VITE_DISPATCHED_MOCK === "1"
    ? import("./engine/mock/transport").then((m) => m.installMock())
    : Promise.resolve();

void boot.then(() => {
  ReactDOM.createRoot(document.getElementById("root")!).render(
    <React.StrictMode>
      <App />
    </React.StrictMode>,
  );
});
