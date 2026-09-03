// Cratebase auth demo — mirrors the official JS SDK's shape 1:1 against the
// default `users` auth collection. Imported via the bare specifier
// "cratebase" (resolved by the import map in index.html/oauth-callback.html
// to the built local package, sdk/js/dist) — zero build step, zero npm publish needed.
import { Cratebase, ClientResponseError } from "cratebase";

const SERVER_URL_KEY = "cratebase_demo_server_url";
const DEFAULT_SERVER_URL = "http://localhost:8090";
const COLLECTION = "users";

function getServerUrl() {
  return localStorage.getItem(SERVER_URL_KEY) || DEFAULT_SERVER_URL;
}

function setServerUrl(url) {
  localStorage.setItem(SERVER_URL_KEY, url);
}

let cb = new Cratebase(getServerUrl());
let users = cb.collection(COLLECTION);

const serverUrlInput = document.getElementById("server-url");
serverUrlInput.value = getServerUrl();
serverUrlInput.addEventListener("change", () => {
  const url = serverUrlInput.value.trim() || DEFAULT_SERVER_URL;
  setServerUrl(url);
  // Re-create the client against the new base URL, preserving the current
  // session (authStore persists to localStorage independently of this).
  cb = new Cratebase(url, cb.authStore);
  users = cb.collection(COLLECTION);
  loadOAuthMethods();
});

// ---------------------------------------------------------------------------
// status helpers
// ---------------------------------------------------------------------------

function showStatus(id, message, kind) {
  const el = document.getElementById(id);
  el.textContent = message;
  el.className = `status ${kind}`;
  el.classList.remove("hidden");
}

function clearStatus(id) {
  const el = document.getElementById(id);
  el.classList.add("hidden");
  el.textContent = "";
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

async function handleForm(formId, statusId, successMessage, action) {
  const form = document.getElementById(formId);
  form.addEventListener("submit", async (event) => {
    event.preventDefault();
    clearStatus(statusId);
    const data = Object.fromEntries(new FormData(form).entries());
    const submitButton = form.querySelector('button[type="submit"]');
    submitButton.disabled = true;
    try {
      await action(data);
      showStatus(statusId, successMessage, "ok");
      form.reset();
    } catch (err) {
      showStatus(statusId, describeError(err), "err");
    } finally {
      submitButton.disabled = false;
    }
  });
}

// ---------------------------------------------------------------------------
// view switching
// ---------------------------------------------------------------------------

function renderAuthState() {
  const guestView = document.getElementById("view-guest");
  const authView = document.getElementById("view-auth");
  const record = cb.authStore.model;

  if (cb.authStore.isValid && record) {
    guestView.classList.add("hidden");
    authView.classList.remove("hidden");
    renderProfile(record);
  } else {
    guestView.classList.remove("hidden");
    authView.classList.add("hidden");
  }
}

function renderProfile(record) {
  const verified = Boolean(record.verified);
  const fields = document.getElementById("profile-fields");
  fields.innerHTML = `
    <div class="field"><span>id</span><span>${escapeHtml(record.id ?? "")}</span></div>
    <div class="field"><span>email</span><span>${escapeHtml(record.email ?? "")}</span></div>
    <div class="field">
      <span>verified</span>
      <span class="pill ${verified ? "verified" : "unverified"}">${verified ? "verified" : "unverified"}</span>
    </div>
  `;

  document.getElementById("card-verification").classList.toggle("hidden", verified);
}

function escapeHtml(value) {
  return String(value).replace(/[&<>"']/g, (ch) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[ch]);
}

cb.authStore.onChange(() => renderAuthState());

// ---------------------------------------------------------------------------
// guest view: register / login / password reset
// ---------------------------------------------------------------------------

handleForm("form-register", "status-register", "Account created — signed in.", async (data) => {
  await users.create({ email: data.email, password: data.password, passwordConfirm: data.passwordConfirm });
  await users.authWithPassword(data.email, data.password);
});

handleForm("form-login", "status-login", "Signed in.", async (data) => {
  await users.authWithPassword(data.identity, data.password);
});

handleForm("form-reset-request", "status-reset-request", "If that email exists, a reset link was sent.", async (data) => {
  await users.requestPasswordReset(data.email);
});

handleForm("form-reset-confirm", "status-reset-confirm", "Password reset — you can log in with the new password.", async (data) => {
  await users.confirmPasswordReset(data.token, data.password, data.passwordConfirm);
});

// ---------------------------------------------------------------------------
// authenticated view: logout / verification / email change
// ---------------------------------------------------------------------------

document.getElementById("btn-logout").addEventListener("click", () => {
  cb.authStore.clear();
});

document.getElementById("btn-request-verification").addEventListener("click", async () => {
  clearStatus("status-request-verification");
  const email = cb.authStore.model?.email;
  if (!email) return;
  try {
    await users.requestVerification(email);
    showStatus("status-request-verification", "Verification email sent.", "ok");
  } catch (err) {
    showStatus("status-request-verification", describeError(err), "err");
  }
});

handleForm("form-confirm-verification", "status-confirm-verification", "Email verified.", async (data) => {
  await users.confirmVerification(data.token);
  await users.authRefresh(); // pull the now-`verified: true` record
});

handleForm("form-request-email-change", "status-request-email-change", "Confirmation link sent to the new address.", async (data) => {
  await users.requestEmailChange(data.newEmail);
});

handleForm("form-confirm-email-change", "status-confirm-email-change", "Email changed.", async (data) => {
  await users.confirmEmailChange(data.token);
  await users.authRefresh(); // pull the updated `email`
});

// ---------------------------------------------------------------------------
// OAuth2
// ---------------------------------------------------------------------------

function oauthRedirectUri() {
  return new URL("oauth-callback.html", window.location.href).toString();
}

const PROVIDER_LABELS = {
  google: "Sign in with Google",
  github: "Sign in with GitHub",
};

async function loadOAuthMethods() {
  const badge = document.getElementById("oauth2-badge");
  const list = document.getElementById("oauth-list");
  clearStatus("status-oauth");
  badge.textContent = "checking…";
  list.innerHTML = "";

  try {
    const methods = await users.listAuthMethods(oauthRedirectUri());
    if (!methods.oauth2.enabled || methods.oauth2.providers.length === 0) {
      badge.textContent = "not configured";
      list.innerHTML = '<p class="hint" style="margin: 0">No OAuth2 providers are configured on the server (see README).</p>';
      return;
    }
    badge.textContent = `${methods.oauth2.providers.length} provider(s)`;
    for (const provider of methods.oauth2.providers) {
      const button = document.createElement("button");
      button.type = "button";
      button.className = "oauth";
      button.textContent = PROVIDER_LABELS[provider.name] || `Sign in with ${provider.name}`;
      button.addEventListener("click", () => {
        // The callback page runs after a full-page redirect and has no
        // memory of which provider's authUrl was clicked, so stash it
        // (plus the exact redirectUri we used to build the authUrl) before
        // navigating away.
        sessionStorage.setItem("cratebase_oauth_provider", provider.name);
        sessionStorage.setItem("cratebase_oauth_redirect_uri", oauthRedirectUri());
        sessionStorage.setItem("cratebase_oauth_server_url", getServerUrl());
        window.location.href = provider.authUrl;
      });
      list.appendChild(button);
    }
  } catch (err) {
    badge.textContent = "error";
    showStatus("status-oauth", describeError(err), "err");
  }
}

// ---------------------------------------------------------------------------
// boot
// ---------------------------------------------------------------------------

renderAuthState();
loadOAuthMethods();
