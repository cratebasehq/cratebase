import { Button, Text } from "@react-email/components";
import { button, EmailLayout, muted, text } from "./layout.js";

export interface EmailChangeEmailProps {
  appName: string;
  newEmail: string;
  actionUrl: string;
  expiresIn: string;
}

export default function EmailChangeEmail({ appName, newEmail, actionUrl, expiresIn }: EmailChangeEmailProps) {
  return (
    <EmailLayout appName={appName} preview={`Confirm your new email for ${appName}`} heading="Confirm your new email">
      <Text style={text}>
        Confirm that {newEmail} is your new sign-in address for {appName}. Your account still uses
        your old address until you confirm.
      </Text>
      <Button href={actionUrl} style={button}>
        Confirm new email
      </Button>
      <Text style={muted}>This link expires in {expiresIn}.</Text>
    </EmailLayout>
  );
}
