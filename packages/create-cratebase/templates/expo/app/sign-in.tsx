import { useState } from "react";
import { Pressable, StyleSheet, Text, TextInput, View } from "react-native";
import { router } from "expo-router";
import { cb } from "@/lib/cratebase";

type Tab = "password" | "otp";

// Password and email-code (OTP) sign-in — the two methods that need no
// extra native setup (deep linking for magic links / OAuth redirects, or a
// QR-rendering library for TOTP). Those all work the same way from Expo as
// from the web templates (`cb.auth.signIn.magicLink`/`.social`/`.totp` —
// see the nextjs template's sign-in screen for a reference implementation)
// — they're just left out here to keep this starter lean.
export default function SignIn() {
  const [tab, setTab] = useState<Tab>("password");
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const [identity, setIdentity] = useState("demo@example.com");
  const [password, setPassword] = useState("password123");

  const [otpEmail, setOtpEmail] = useState("");
  const [otpId, setOtpId] = useState<string | null>(null);
  const [otpCode, setOtpCode] = useState("");

  async function signInWithPassword() {
    setPending(true);
    setError(null);
    try {
      await cb.auth.signIn.password({ identity, password });
      router.replace("/notes");
    } catch (err) {
      setError(err instanceof Error ? err.message : "Sign-in failed.");
    } finally {
      setPending(false);
    }
  }

  async function requestOtp() {
    setPending(true);
    setError(null);
    try {
      const { otpId } = await cb.auth.otp.request({ email: otpEmail });
      setOtpId(otpId);
    } catch (err) {
      setError(err instanceof Error ? err.message : "Couldn't send a code.");
    } finally {
      setPending(false);
    }
  }

  async function confirmOtp() {
    if (!otpId) return;
    setPending(true);
    setError(null);
    try {
      await cb.auth.signIn.otp({ otpId, code: otpCode });
      router.replace("/notes");
    } catch (err) {
      setError(err instanceof Error ? err.message : "That code didn't work.");
    } finally {
      setPending(false);
    }
  }

  return (
    <View style={styles.screen}>
      <Text style={styles.title}>{"{{PROJECT_NAME}}"}</Text>
      <Text style={styles.subtitle}>Sign in to continue</Text>

      <View style={styles.tabs}>
        {(["password", "otp"] as const).map((t) => (
          <Pressable
            key={t}
            style={[styles.tab, tab === t && styles.tabActive]}
            onPress={() => {
              setTab(t);
              setError(null);
            }}
          >
            <Text style={tab === t ? styles.tabTextActive : styles.tabText}>
              {t === "password" ? "Password" : "Email code"}
            </Text>
          </Pressable>
        ))}
      </View>

      {tab === "password" && (
        <View style={styles.form}>
          <TextInput
            style={styles.input}
            placeholder="you@example.com"
            autoCapitalize="none"
            keyboardType="email-address"
            value={identity}
            onChangeText={setIdentity}
          />
          <TextInput
            style={styles.input}
            placeholder="Password"
            secureTextEntry
            value={password}
            onChangeText={setPassword}
          />
          <Pressable style={styles.button} onPress={signInWithPassword} disabled={pending}>
            <Text style={styles.buttonText}>{pending ? "Signing in…" : "Sign in"}</Text>
          </Pressable>
        </View>
      )}

      {tab === "otp" &&
        (otpId === null ? (
          <View style={styles.form}>
            <TextInput
              style={styles.input}
              placeholder="you@example.com"
              autoCapitalize="none"
              keyboardType="email-address"
              value={otpEmail}
              onChangeText={setOtpEmail}
            />
            <Pressable style={styles.button} onPress={requestOtp} disabled={pending}>
              <Text style={styles.buttonText}>{pending ? "Sending…" : "Send me a code"}</Text>
            </Pressable>
          </View>
        ) : (
          <View style={styles.form}>
            <Text style={styles.hint}>
              We sent a code to {otpEmail}. In dev, check the mail inbox printed by `cratebase dev`.
            </Text>
            <TextInput
              style={styles.input}
              placeholder="123456"
              keyboardType="number-pad"
              value={otpCode}
              onChangeText={setOtpCode}
            />
            <Pressable style={styles.button} onPress={confirmOtp} disabled={pending}>
              <Text style={styles.buttonText}>{pending ? "Verifying…" : "Verify code"}</Text>
            </Pressable>
          </View>
        ))}

      {error && <Text style={styles.error}>{error}</Text>}

      <Text style={styles.hint}>Demo login: demo@example.com / password123</Text>
    </View>
  );
}

const styles = StyleSheet.create({
  screen: { flex: 1, justifyContent: "center", padding: 24, gap: 12 },
  title: { fontSize: 28, fontWeight: "700" },
  subtitle: { fontSize: 14, color: "#666", marginBottom: 12 },
  tabs: { flexDirection: "row", backgroundColor: "#eee", borderRadius: 10, padding: 4, marginBottom: 12 },
  tab: { flex: 1, paddingVertical: 8, borderRadius: 8, alignItems: "center" },
  tabActive: { backgroundColor: "#fff" },
  tabText: { color: "#666" },
  tabTextActive: { color: "#111", fontWeight: "600" },
  form: { gap: 10 },
  input: {
    borderWidth: 1,
    borderColor: "#ddd",
    borderRadius: 10,
    paddingHorizontal: 14,
    paddingVertical: 12,
    fontSize: 16,
  },
  button: {
    backgroundColor: "#e8622c",
    borderRadius: 10,
    paddingVertical: 12,
    alignItems: "center",
    marginTop: 4,
  },
  buttonText: { color: "#fff", fontWeight: "600", fontSize: 16 },
  error: { color: "#d33", marginTop: 8 },
  hint: { color: "#888", fontSize: 12, marginTop: 16, textAlign: "center" },
});
