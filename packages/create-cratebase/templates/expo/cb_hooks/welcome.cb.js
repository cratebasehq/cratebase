// Sends the built-in "welcome" email template (seeded automatically on
// every fresh Cratebase instance) to a user right after they sign up.
//
// This runs as an after-success hook, inside `$mails.send`'s own deferred
// delivery (queued until the signup's transaction commits), so it can
// never block or fail the signup request itself — see
// https://cratebase.dev/docs/email/templates/ and
// https://cratebase.dev/docs/extending/js-hooks/.
onRecordAfterCreateSuccess((e) => {
  try {
    $mails.send({
      to: [{ address: e.record.getString("email"), name: e.record.getString("name") }],
      template: "welcome",
      data: { name: e.record.getString("name") || e.record.getString("email") },
    });
  } catch (err) {
    $app.logger().error("welcome email failed", "user", e.record.id, "error", String(err));
  }
}, "users");
