// OAuth2 redirect target. The provider (Google/GitHub) sends the browser
// here with `?code=...` after the user approves the consent screen; this
// exchanges that code for a Cratebase session and bounces back to the app.
//
// Note: the server's OAuth2 routes are still being implemented — this
// exchange will 404 against a current Cratebase build. The client-side
// code below is written to be correct once they land (see README).
import PocketBase, { ClientResponseError } from "pocketbase";

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

  // The provider we clicked, its PKCE codeVerifier, and the exact
  // redirectUri used to build its authUrl were stashed in sessionStorage
  // right before navigating here — this page has no other way to know
  // which provider issued `code`, or to complete the PKCE exchange.
  const provider = sessionStorage.getItem("cratebase_oauth_provider");
  const codeVerifier = sessionStorage.getItem("cratebase_oauth_code_verifier");
  const redirectUri = sessionStorage.getItem("cratebase_oauth_redirect_uri");

  if (oauthError) {
    setStatus(`Provider denied the request: ${oauthError}`, "err");
    return;
  }
  if (!code) {
    setStatus("Missing `code` query parameter — this page must be reached via an OAuth2 redirect.", "err");
    return;
  }
  if (!provider || !codeVerifier || !redirectUri) {
    setStatus("Missing stashed provider/codeVerifier/redirectUri — start the OAuth2 flow again from the app.", "err");
    return;
  }

  try {
    const cb = new PocketBase(getServerUrl());
    await cb.collection(COLLECTION).authWithOAuth2Code(provider, code, codeVerifier, redirectUri);
    // authWithOAuth2Code already saved the token/record into `cb.authStore`,
    // which persists to localStorage — index.html picks it up on load.
    sessionStorage.removeItem("cratebase_oauth_provider");
    sessionStorage.removeItem("cratebase_oauth_code_verifier");
    sessionStorage.removeItem("cratebase_oauth_redirect_uri");
    sessionStorage.removeItem("cratebase_oauth_server_url");
    setStatus("Signed in — redirecting…", "ok");
    window.location.href = "index.html";
  } catch (err) {
    setStatus(describeError(err), "err");
  }
}

run();
