// Registers a minimal DOM (window/document/etc.) globally so hooks that
// touch it indirectly through React's DOM renderer (`@testing-library/react`)
// work under `bun test`, which otherwise runs in a plain Node-like
// environment with no DOM at all.
import { GlobalRegistrator } from "@happy-dom/global-registrator";

// `url` gives every test a real (non-`about:blank`, non-`null`-origin)
// starting location, so `window.history.pushState`/`window.location.*`
// work the way they do in a real browser — needed by
// `useMagicLinkCallback.test.tsx`, and harmless for every other test,
// which never touches location/history.
GlobalRegistrator.register({ url: "http://localhost/" });
