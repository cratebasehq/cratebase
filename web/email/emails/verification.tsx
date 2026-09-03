import { Button, Text } from "@react-email/components";
import { button, EmailLayout, muted, text } from "./layout.js";

export interface VerificationEmailProps {
  appName: string;
  actionUrl: string;
  expiresIn: string;
}

export default function VerificationEmail({ appName, actionUrl, expiresIn }: VerificationEmailProps) {
  return (
    <EmailLayout appName={appName} preview={`Confirm your email for ${appName}`} heading="Confirm your email">
      <Text style={text}>
        Confirm this address to finish setting up your {appName} account.
      </Text>
      <Button href={actionUrl} style={button}>
        Verify email
      </Button>
      <Text style={muted}>This link expires in {expiresIn}.</Text>
    </EmailLayout>
  );
}
