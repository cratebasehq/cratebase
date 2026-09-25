import React from "react";
import ReactDOM from "react-dom/client";
import { CratebaseProvider } from "@cratebase/react";
import { cb } from "./cratebase.js";
import { App } from "./App.js";
import "./index.css";

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <CratebaseProvider client={cb}>
      <App />
    </CratebaseProvider>
  </React.StrictMode>,
);
