import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { BrowserRouter } from "react-router-dom";
import { CratebaseProvider } from "@cratebase/react";
import { cb } from "@/lib/cratebase";
import { App } from "./App";
import "./index.css";

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <CratebaseProvider client={cb}>
      <BrowserRouter>
        <App />
      </BrowserRouter>
    </CratebaseProvider>
  </StrictMode>,
);
