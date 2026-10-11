import { useState } from "react";
import { KeyboardAvoidingView, Platform, ScrollView, StyleSheet, Text, View } from "react-native";
import { Link, router } from "expo-router";
import { cb } from "@/lib/cratebase";
import { useTheme } from "@/lib/theme";
import { Button } from "@/components/Button";
import { TextField } from "@/components/TextField";

export default function SignUp() {
  const theme = useTheme();
  const [name, setName] = useState("");
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function submit() {
    setPending(true);
    setError(null);
    try {
      // Creates the account, then signs in.
      await cb.auth.signUp({ email: email.trim(), password, passwordConfirm: password, name });
      router.replace("/notes");
    } catch (err) {
      setError(err instanceof Error ? err.message : "Sign-up failed.");
    } finally {
      setPending(false);
    }
  }

  return (
    <KeyboardAvoidingView style={styles.flex} behavior={Platform.OS === "ios" ? "padding" : undefined}>
      <ScrollView contentContainerStyle={styles.screen} keyboardShouldPersistTaps="handled">
        <Text accessibilityRole="header" style={[styles.title, { color: theme.text }]}>
          Create an account
        </Text>
        <Text style={[styles.subtitle, { color: theme.muted }]}>
          Already have one?{" "}
          <Link href="/sign-in" style={{ color: theme.link }}>
            Sign in
          </Link>
        </Text>

        <View style={styles.form}>
          <TextField label="Name" autoComplete="name" value={name} onChangeText={setName} />
          <TextField
            label="Email"
            autoCapitalize="none"
            autoComplete="email"
            keyboardType="email-address"
            placeholder="you@example.com"
            value={email}
            onChangeText={setEmail}
          />
          <TextField
            label="Password (8+ characters)"
            autoComplete="new-password"
            secureTextEntry
            value={password}
            onChangeText={setPassword}
            onSubmitEditing={submit}
          />
          {error && (
            <Text accessibilityRole="alert" style={{ color: theme.danger }}>
              {error}
            </Text>
          )}
          <Button title="Create account" onPress={submit} loading={pending} disabled={!email || password.length < 8} />
        </View>
      </ScrollView>
    </KeyboardAvoidingView>
  );
}

const styles = StyleSheet.create({
  flex: { flex: 1 },
  screen: { flexGrow: 1, justifyContent: "center", padding: 24, gap: 8 },
  title: { fontSize: 28, fontWeight: "700" },
  subtitle: { fontSize: 14, marginBottom: 16 },
  form: { gap: 14 },
});
