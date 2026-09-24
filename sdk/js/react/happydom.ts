// Registers a minimal DOM (window/document/etc.) globally so hooks that
// touch it indirectly through React's DOM renderer (`@testing-library/react`)
// work under `bun test`, which otherwise runs in a plain Node-like
// environment with no DOM at all.
import { GlobalRegistrator } from "@happy-dom/global-registrator";

GlobalRegistrator.register();
