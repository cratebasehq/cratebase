import { createRoot } from "react-dom/client";
import { App } from "./App";
import "./styles.css";

// No StrictMode: it double-invokes effects in dev, which can race the
// presence hook's create-or-update upsert into creating two records for
// the same tab. Not worth the noise in a small demo app.
createRoot(document.getElementById("root")!).render(<App />);
