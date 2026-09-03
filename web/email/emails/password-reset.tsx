import { Button, Text } from "@react-email/components";
import { button, EmailLayout, muted, text } from "./layout.js";

export interface PasswordResetEmailProps {
  appName: string;
  actionUrl: string;
  expiresIn: string;
}

export default function PasswordResetEmail({ appName, actionUrl, expiresIn }: PasswordResetEmailProps) {
  return (
    <EmailLayout appName={appName} preview={`Reset your ${appName} password`} heading="Reset your password">
      <Text style={text}>
        We received a request to reset the password on your {appName} account.
      </Text>
      <Button href={actionUrl} style={button}>
        Reset password
      </Button>
      <Text style={muted}>This link expires in {expiresIn}.</Text>
    </EmailLayout>
  );
}
