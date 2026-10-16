import "./ui/styles.css";
import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { applyTheme, resolveTheme } from "./ui";

// Apply the theme before the first render so the first frame already has the variables.
applyTheme(resolveTheme("system", window.matchMedia("(prefers-color-scheme: dark)").matches), "compact");

const boot =
  import.meta.env.VITE_DISPATCHED_MOCK === "1"
    ? import("./engine/mock/transport").then(async (m) => {
        m.installMock();
        // Mock builds only (screenshots and e2e): lets a script put a view in a state no control here reaches yet.
        const v = await import("./shell/messages/viewStore");
        (window as unknown as { __dispatchedMock?: unknown }).__dispatchedMock = { setViewState: v.setViewState, viewKey: v.viewKey };
      })
    : Promise.resolve();

void boot.then(() => {
  ReactDOM.createRoot(document.getElementById("root")!).render(
    <React.StrictMode>
      <App />
    </React.StrictMode>,
  );
});
