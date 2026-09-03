import { mkdir, writeFile } from "node:fs/promises";
import { render } from "@react-email/render";
import VerificationEmail from "../emails/verification.js";
import PasswordResetEmail from "../emails/password-reset.js";
import EmailChangeEmail from "../emails/email-change.js";
import OtpEmail from "../emails/otp.js";

// Rendered with literal `{{token}}` placeholders as props — react-email
// just outputs them as plain HTML text, and `crates/server/src/mail.rs`
// does a dumb string `.replace()` on the real values before sending. No
// Node runtime needed at request time; this build step runs once
// (`bun run email:build`, mirroring `admin:build`) and its output is
// embedded into the `cratebase` binary via `rust-embed`.
const templates = {
  "verification.html": (
    <VerificationEmail appName="{{appName}}" actionUrl="{{actionUrl}}" expiresIn="{{expiresIn}}" />
  ),
  "password-reset.html": (
    <PasswordResetEmail appName="{{appName}}" actionUrl="{{actionUrl}}" expiresIn="{{expiresIn}}" />
  ),
  "email-change.html": (
    <EmailChangeEmail
      appName="{{appName}}"
      newEmail="{{newEmail}}"
      actionUrl="{{actionUrl}}"
      expiresIn="{{expiresIn}}"
    />
  ),
  "otp.html": (
    <OtpEmail appName="{{appName}}" code="{{code}}" expiresIn="{{expiresIn}}" />
  ),
};

await mkdir(new URL("../dist", import.meta.url), { recursive: true });

for (const [filename, element] of Object.entries(templates)) {
  const html = await render(element, { pretty: false });
  await writeFile(new URL(`../dist/${filename}`, import.meta.url), html, "utf8");
  console.log(`built ${filename}`);
}
