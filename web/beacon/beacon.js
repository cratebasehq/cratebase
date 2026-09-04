/*!
 * Cratebase analytics beacon — self-hosted, cookie-free pageview counter.
 *
 * Drop this in a `<script>` tag on any site:
 *
 *   <script defer src="https://your-cratebase.example.com/beacon.js"
 *           data-api="https://your-cratebase.example.com"></script>
 *
 * `data-api` is the origin of the Cratebase instance to send pageviews to;
 * it defaults to this script's own origin, so if you serve beacon.js
 * straight from your Cratebase deployment (copy it into a static host, or
 * proxy it) you can drop the attribute entirely.
 *
 * Fires once per page load: `POST {api}/api/collections/analytics/records`
 * with `{url, referrer, path}`. No new server endpoint is involved — the
 * `analytics` collection's own `createRule` (public, see
 * analytics-collection.json) is what allows this unauthenticated write.
 *
 * Deliberately does NOT set a cookie, touch localStorage/sessionStorage, or
 * send anything that could be replayed as a persistent visitor identifier.
 * That also means it cannot count unique visitors — see the dashboard's
 * Analytics page for what that trade-off means in practice.
 */
(function () {
  "use strict";
  var script =
    document.currentScript ||
    (function () {
      var scripts = document.getElementsByTagName("script");
      return scripts[scripts.length - 1];
    })();
  var api = (script && script.getAttribute("data-api")) || (script && script.src ? new URL(script.src).origin : "");
  if (!api) return;

  var payload = JSON.stringify({
    url: location.href,
    referrer: document.referrer || "",
    path: location.pathname,
  });
  var endpoint = api.replace(/\/$/, "") + "/api/collections/analytics/records";

  if (navigator.sendBeacon) {
    navigator.sendBeacon(endpoint, new Blob([payload], { type: "application/json" }));
  } else {
    fetch(endpoint, { method: "POST", headers: { "content-type": "application/json" }, body: payload, keepalive: true }).catch(
      function () {},
    );
  }
})();
