import { useState } from "react";
import { KeyboardAvoidingView, Platform, Pressable, ScrollView, StyleSheet, Text, View } from "react-native";
import { Link, router } from "expo-router";
import { cb } from "@/lib/cratebase";
import { useTheme } from "@/lib/theme";
import { Button } from "@/components/Button";
import { TextField } from "@/components/TextField";

type Tab = "password" | "otp";

// Password and email-code (OTP) sign-in — the two methods that need no
// extra native setup (deep linking for magic links / OAuth redirects, or a
// QR-rendering library for TOTP). Those all work the same way from Expo as
// from the web templates (`cb.auth.signIn.magicLink`/`.social`/`.totp` —
// see the nextjs template's sign-in screen for a reference implementation)
// — they're just left out here to keep this starter lean.
export default function SignIn() {
  const theme = useTheme();
  const [tab, setTab] = useState<Tab>("password");
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const [identity, setIdentity] = useState("");
  const [password, setPassword] = useState("");

  const [otpEmail, setOtpEmail] = useState("");
  const [otpId, setOtpId] = useState<string | null>(null);
  const [otpCode, setOtpCode] = useState("");

  async function attempt(action: () => Promise<void>, fallback: string) {
    setPending(true);
    setError(null);
    try {
      await action();
    } catch (err) {
      setError(err instanceof Error ? err.message : fallback);
    } finally {
      setPending(false);
    }
  }

  const signInWithPassword = () =>
    attempt(async () => {
      await cb.auth.signIn.password({ identity: identity.trim(), password });
      router.replace("/notes");
    }, "Sign-in failed.");

  const requestOtp = () =>
    attempt(async () => {
      const { otpId } = await cb.auth.otp.request({ email: otpEmail.trim() });
      setOtpId(otpId);
    }, "Couldn't send a code.");

  const confirmOtp = () =>
    attempt(async () => {
      if (!otpId) return;
      await cb.auth.signIn.otp({ otpId, code: otpCode.trim() });
      router.replace("/notes");
    }, "That code didn't work.");

  return (
    <KeyboardAvoidingView style={styles.flex} behavior={Platform.OS === "ios" ? "padding" : undefined}>
      <ScrollView contentContainerStyle={styles.screen} keyboardShouldPersistTaps="handled">
        <Text accessibilityRole="header" style={[styles.title, { color: theme.text }]}>
          {"{{PROJECT_NAME}}"}
        </Text>
        <Text style={[styles.subtitle, { color: theme.muted }]}>
          Sign in to continue, or{" "}
          <Link href="/sign-up" style={{ color: theme.link }}>
            create an account
          </Link>
        </Text>

        <View style={[styles.tabs, { backgroundColor: theme.border }]} accessibilityRole="tablist">
          {(["password", "otp"] as const).map((t) => (
            <Pressable
              key={t}
              accessibilityRole="tab"
              accessibilityState={{ selected: tab === t }}
              style={[styles.tab, tab === t && { backgroundColor: theme.card }]}
              onPress={() => {
                setTab(t);
                setError(null);
              }}
            >
              <Text style={{ color: tab === t ? theme.text : theme.muted, fontWeight: tab === t ? "600" : "400" }}>
                {t === "password" ? "Password" : "Email code"}
              </Text>
            </Pressable>
          ))}
        </View>

        {tab === "password" && (
          <View style={styles.form}>
            <TextField
              label="Email"
              autoCapitalize="none"
              autoComplete="email"
              keyboardType="email-address"
              placeholder="you@example.com"
              value={identity}
              onChangeText={setIdentity}
            />
            <TextField
              label="Password"
              autoComplete="current-password"
              secureTextEntry
              value={password}
              onChangeText={setPassword}
              onSubmitEditing={signInWithPassword}
            />
            <Button title="Sign in" onPress={signInWithPassword} loading={pending} disabled={!identity || !password} />
            {__DEV__ && (
              <Pressable
                onPress={() => {
                  setIdentity("demo@example.com");
                  setPassword("password123");
                }}
              >
                <Text style={[styles.hint, { color: theme.muted }]}>Fill in the seeded demo account</Text>
              </Pressable>
            )}
          </View>
        )}

        {tab === "otp" &&
          (otpId === null ? (
            <View style={styles.form}>
              <TextField
                label="Email"
                autoCapitalize="none"
                autoComplete="email"
                keyboardType="email-address"
                placeholder="you@example.com"
                value={otpEmail}
                onChangeText={setOtpEmail}
              />
              <Button title="Send me a code" onPress={requestOtp} loading={pending} disabled={!otpEmail} />
            </View>
          ) : (
            <View style={styles.form}>
              <Text style={{ color: theme.muted }}>
                We sent a code to {otpEmail}. In dev, open the Mail inbox in the Cratebase dashboard
                (localhost:8090/_/).
              </Text>
              <TextField
                label="Code"
                autoComplete="one-time-code"
                keyboardType="number-pad"
                placeholder="123456"
                value={otpCode}
                onChangeText={setOtpCode}
              />
              <Button title="Verify code" onPress={confirmOtp} loading={pending} disabled={!otpCode} />
            </View>
          ))}

        {error && (
          <Text accessibilityRole="alert" style={{ color: theme.danger, marginTop: 8 }}>
            {error}
          </Text>
        )}
      </ScrollView>
    </KeyboardAvoidingView>
  );
}

const styles = StyleSheet.create({
  flex: { flex: 1 },
  screen: { flexGrow: 1, justifyContent: "center", padding: 24, gap: 8 },
  title: { fontSize: 28, fontWeight: "700" },
  subtitle: { fontSize: 14, marginBottom: 12 },
  tabs: { flexDirection: "row", borderRadius: 10, padding: 4, marginBottom: 12 },
  tab: { flex: 1, paddingVertical: 8, borderRadius: 8, alignItems: "center" },
  form: { gap: 14 },
  hint: { fontSize: 12, textAlign: "center", textDecorationLine: "underline" },
});
