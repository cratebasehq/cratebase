import { Text } from "@react-email/components";
import { EmailLayout, muted, text } from "./layout.js";

export interface OtpEmailProps {
  appName: string;
  code: string;
  expiresIn: string;
}

const codeBlock: React.CSSProperties = {
  fontSize: "32px",
  fontWeight: 700,
  letterSpacing: "8px",
  textAlign: "center",
  margin: "24px 0",
  fontFamily: "monospace",
};

export default function OtpEmail({ appName, code, expiresIn }: OtpEmailProps) {
  return (
    <EmailLayout appName={appName} preview={`Your ${appName} verification code`} heading="Your verification code">
      <Text style={text}>
        Enter this code to finish signing in to {appName}.
      </Text>
      <Text style={codeBlock}>{code}</Text>
      <Text style={muted}>This code expires in {expiresIn}. If you didn't request it, you can ignore this email.</Text>
    </EmailLayout>
  );
}
