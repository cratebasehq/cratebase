import type { JSONContent } from "@tiptap/core";
import {
  Bell,
  KeyRound,
  Link2,
  Mail,
  Newspaper,
  Receipt,
  Sparkles,
  SquareDashedMousePointer,
} from "lucide-react";
import type { ComponentType } from "react";

// --- tiny JSONContent builders, just to keep the templates below
// readable — no magic, just less punctuation per node. ---

function text(value: string): JSONContent {
  return { type: "text", text: value };
}

function variable(path: string): JSONContent {
  return { type: "variable", attrs: { path } };
}

function heading(level: 1 | 2 | 3, ...children: JSONContent[]): JSONContent {
  return { type: "heading", attrs: { level }, content: children };
}

function paragraph(...children: JSONContent[]): JSONContent {
  return { type: "paragraph", content: children };
}

function button(label: string, href: string): JSONContent {
  return {
    type: "button",
    attrs: { href, class: "button", alignment: "center" },
    content: [text(label)],
  };
}

function divider(): JSONContent {
  return { type: "horizontalRule" };
}

function doc(...children: JSONContent[]): JSONContent {
  return { type: "doc", content: children };
}

export interface StarterTemplate {
  id: string;
  name: string;
  description: string;
  icon: ComponentType<{ className?: string }>;
  subject: string;
  content: JSONContent;
}

/**
 * The "New template" starter gallery: Blank plus eight well-designed
 * documents to start from and customize, in the same restrained
 * Linear/Vercel/Stripe spirit as the Rust-side redesign
 * (`crates/mailer/src/template.rs`) — short copy, one clear call to
 * action, `{{path}}` variables (via the `variable` node,
 * `./variable-extension`) wherever a real send would need one.
 */
export const STARTER_TEMPLATES: StarterTemplate[] = [
  {
    id: "blank",
    name: "Blank",
    description: "Start from an empty canvas.",
    icon: SquareDashedMousePointer,
    subject: "",
    content: doc(paragraph()),
  },
  {
    id: "welcome",
    name: "Welcome",
    description: "Greet a new user and point them at one next step.",
    icon: Sparkles,
    subject: "Welcome to {{appName}}",
    content: doc(
      heading(1, text("Welcome, "), variable("user.name")),
      paragraph(
        text(
          "We're glad you're here. {{appName}} is ready whenever you are — click below to get started.",
        ),
      ),
      button("Get started", "{{appUrl}}"),
      paragraph(text("Questions? Just reply to this email — we're happy to help.")),
    ),
  },
  {
    id: "verify-email",
    name: "Verify email",
    description: "Confirm a new email address with a single button.",
    icon: Mail,
    subject: "Verify your email for {{appName}}",
    content: doc(
      heading(1, text("Verify your email")),
      paragraph(
        text("Welcome to {{appName}}. Click the button below to verify your email address."),
      ),
      button("Verify email", "{{appUrl}}/_/#/auth/confirm-verification/{{token}}"),
      paragraph(
        text("If you didn't create an account with {{appName}}, you can safely ignore this email."),
      ),
    ),
  },
  {
    id: "reset-password",
    name: "Reset password",
    description: "Send a password-reset link.",
    icon: KeyRound,
    subject: "Reset your {{appName}} password",
    content: doc(
      heading(1, text("Reset your password")),
      paragraph(
        text(
          "We received a request to reset the password for your {{appName}} account. Click the button below to choose a new one.",
        ),
      ),
      button("Reset password", "{{appUrl}}/_/#/auth/confirm-password-reset/{{token}}"),
      paragraph(
        text(
          "This link will expire soon. If you didn't request this, your password won't change.",
        ),
      ),
    ),
  },
  {
    id: "otp-code",
    name: "OTP code",
    description: "A one-time code for sign-in or verification.",
    icon: KeyRound,
    subject: "Your {{appName}} verification code",
    content: doc(
      heading(1, text("Your verification code")),
      paragraph(text("Enter this code to continue signing in to {{appName}}:")),
      heading(2, variable("otp")),
      paragraph(text("This code will expire shortly. If you didn't request it, ignore this email.")),
    ),
  },
  {
    id: "magic-link",
    name: "Magic link",
    description: "Passwordless sign-in with one link.",
    icon: Link2,
    subject: "Your sign-in link for {{appName}}",
    content: doc(
      heading(1, text("Sign in to "), variable("appName")),
      paragraph(text("Click the button below to sign in — no password needed.")),
      button("Sign in", "{{magicLink}}"),
      paragraph(text("This link expires shortly and can only be used once.")),
    ),
  },
  {
    id: "receipt",
    name: "Receipt",
    description: "Confirm a payment with the essentials, no clutter.",
    icon: Receipt,
    subject: "Your {{appName}} receipt",
    content: doc(
      heading(1, text("Thanks for your purchase")),
      paragraph(text("Here's a summary of your order for "), variable("order.id"), text(".")),
      divider(),
      paragraph(text("Amount: "), variable("order.total")),
      paragraph(text("Date: "), variable("order.date")),
      divider(),
      button("View receipt", "{{appUrl}}/receipts/{{order.id}}"),
    ),
  },
  {
    id: "notification",
    name: "Notification",
    description: "A short heads-up with an optional link to follow up.",
    icon: Bell,
    subject: "{{appName}}: new activity on your account",
    content: doc(
      heading(1, text("New activity")),
      paragraph(variable("notification.message")),
      button("View details", "{{appUrl}}"),
    ),
  },
  {
    id: "newsletter",
    name: "Newsletter",
    description: "A two-column layout for a digest or announcement.",
    icon: Newspaper,
    subject: "{{appName}} — this week",
    content: doc(
      heading(1, text("This week at "), variable("appName")),
      paragraph(text("A quick roundup of what's new.")),
      {
        type: "twoColumns",
        content: [
          {
            type: "columnsColumn",
            content: [heading(3, text("Update one")), paragraph(text("A short summary goes here."))],
          },
          {
            type: "columnsColumn",
            content: [heading(3, text("Update two")), paragraph(text("Another short summary."))],
          },
        ],
      },
      button("Read more", "{{appUrl}}"),
    ),
  },
];

export const BLANK_STARTER_TEMPLATE = STARTER_TEMPLATES[0];
