// OAuth2 redirect target. The provider (Google/GitHub) sends the browser
// here with `?code=...` after the user approves the consent screen; this
// exchanges that code for a Cratebase session and bounces back to the app.
import { Cratebase, ClientResponseError } from "cratebase";

const SERVER_URL_KEY = "cratebase_demo_server_url";
const DEFAULT_SERVER_URL = "http://localhost:8090";
const COLLECTION = "users";

function getServerUrl() {
  // Prefer the server URL the demo used when it built the authUrl (stashed
  // right before the redirect) so this page works even if opened fresh
  // (e.g. a new tab) without its own localStorage state yet.
  return sessionStorage.getItem("cratebase_oauth_server_url") || localStorage.getItem(SERVER_URL_KEY) || DEFAULT_SERVER_URL;
}

function setStatus(message, kind) {
  const el = document.getElementById("callback-status");
  el.textContent = message;
  el.className = `status ${kind}`;
}

function describeError(err) {
  if (err instanceof ClientResponseError) {
    const fields = Object.entries(err.data || {})
      .map(([field, info]) => `${field}: ${info.message}`)
      .join("\n");
    return fields ? `${err.message}\n${fields}` : err.message;
  }
  return err?.message || String(err);
}

async function run() {
  const params = new URLSearchParams(window.location.search);
  const oauthError = params.get("error");
  const code = params.get("code");

  // The provider we clicked and the exact redirectUri used to build its
  // authUrl were stashed in sessionStorage right before navigating here —
  // this page has no other way to know which provider issued `code`.
  const provider = sessionStorage.getItem("cratebase_oauth_provider");
  const redirectUri = sessionStorage.getItem("cratebase_oauth_redirect_uri");

  if (oauthError) {
    setStatus(`Provider denied the request: ${oauthError}`, "err");
    return;
  }
  if (!code) {
    setStatus("Missing `code` query parameter — this page must be reached via an OAuth2 redirect.", "err");
    return;
  }
  if (!provider || !redirectUri) {
    setStatus("Missing stashed provider/redirectUri — start the OAuth2 flow again from the app.", "err");
    return;
  }

  try {
    const cb = new Cratebase(getServerUrl());
    await cb.collection(COLLECTION).authWithOAuth2(provider, code, redirectUri);
    // `authWithOAuth2` already saved the token/record into `cb.authStore`,
    // which persists to localStorage — index.html picks it up on load.
    sessionStorage.removeItem("cratebase_oauth_provider");
    sessionStorage.removeItem("cratebase_oauth_redirect_uri");
    sessionStorage.removeItem("cratebase_oauth_server_url");
    setStatus("Signed in — redirecting…", "ok");
    window.location.href = "index.html";
  } catch (err) {
    setStatus(describeError(err), "err");
  }
}

run();
