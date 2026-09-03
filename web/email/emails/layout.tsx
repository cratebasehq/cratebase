import {
  Body,
  Container,
  Head,
  Heading,
  Hr,
  Html,
  Preview,
  Section,
  Text,
} from "@react-email/components";
import type { ReactNode } from "react";

interface EmailLayoutProps {
  preview: string;
  appName: string;
  heading: string;
  children: ReactNode;
}

/** Shared shell for every transactional email: calm, generous whitespace,
 * one accent color, a single clear action — the same restraint the admin
 * dashboard's design language aims for. */
export function EmailLayout({ preview, appName, heading, children }: EmailLayoutProps) {
  return (
    <Html>
      <Head />
      <Preview>{preview}</Preview>
      <Body style={body}>
        <Container style={container}>
          <Text style={brand}>{appName}</Text>
          <Section style={card}>
            <Heading style={h1}>{heading}</Heading>
            {children}
          </Section>
          <Hr style={hr} />
          <Text style={footer}>
            If you didn't request this, you can safely ignore this email.
          </Text>
        </Container>
      </Body>
    </Html>
  );
}

const body: React.CSSProperties = {
  backgroundColor: "#f4f4f5",
  fontFamily:
    '-apple-system,BlinkMacSystemFont,"Segoe UI",Roboto,Helvetica,Arial,sans-serif',
  margin: 0,
  padding: "40px 0",
};

const container: React.CSSProperties = {
  maxWidth: "480px",
  margin: "0 auto",
  padding: "0 24px",
};

const brand: React.CSSProperties = {
  fontSize: "13px",
  fontWeight: 600,
  letterSpacing: "0.04em",
  textTransform: "uppercase",
  color: "#71717a",
  textAlign: "center",
  marginBottom: "24px",
};

const card: React.CSSProperties = {
  backgroundColor: "#ffffff",
  borderRadius: "16px",
  padding: "40px 32px",
  boxShadow: "0 1px 3px rgba(0,0,0,0.06)",
};

const h1: React.CSSProperties = {
  fontSize: "20px",
  fontWeight: 600,
  color: "#18181b",
  margin: "0 0 16px",
};

const hr: React.CSSProperties = {
  borderColor: "#e4e4e7",
  margin: "32px 0 16px",
};

const footer: React.CSSProperties = {
  fontSize: "12.5px",
  color: "#a1a1aa",
  textAlign: "center",
  lineHeight: "1.5",
};

export const text: React.CSSProperties = {
  fontSize: "14.5px",
  lineHeight: "1.6",
  color: "#3f3f46",
  margin: "0 0 24px",
};

export const button: React.CSSProperties = {
  backgroundColor: "#18181b",
  borderRadius: "9px",
  color: "#ffffff",
  fontSize: "14px",
  fontWeight: 600,
  textDecoration: "none",
  textAlign: "center",
  display: "inline-block",
  padding: "12px 24px",
};

export const muted: React.CSSProperties = {
  fontSize: "12.5px",
  color: "#a1a1aa",
  margin: "20px 0 0",
};
