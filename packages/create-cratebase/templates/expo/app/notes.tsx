import { useState } from "react";
import {
  ActivityIndicator,
  Alert,
  FlatList,
  KeyboardAvoidingView,
  Platform,
  Pressable,
  RefreshControl,
  StyleSheet,
  Text,
  TextInput,
  View,
} from "react-native";
import { Redirect, router } from "expo-router";
import { useSafeAreaInsets } from "react-native-safe-area-context";
import type { RecordModel } from "@cratebase/client";
import { useAuth, useRecords, useMutation } from "@/lib/cratebase";
import { useTheme } from "@/lib/theme";
import type { UsersRecord } from "../cratebase-types";

export default function Notes() {
  const theme = useTheme();
  const insets = useSafeAreaInsets();
  const { user, isValid, isLoading, signOut } = useAuth<UsersRecord & RecordModel>();

  // `realtime: true` (the default) refetches this list on every matching
  // create/update/delete event — every open device stays in sync with no
  // manual invalidation. See @cratebase/react's useRecords doc comment.
  const { records: posts, loading, error, refresh } = useRecords("posts", { sort: "-created", realtime: true });
  const { create, remove, pending } = useMutation("posts");
  const [title, setTitle] = useState("");
  const [actionError, setActionError] = useState<string | null>(null);

  if (isLoading) return null;
  if (!isValid) return <Redirect href="/sign-in" />;

  async function addNote() {
    if (!title.trim() || !user) return;
    setActionError(null);
    try {
      await create({ title: title.trim(), owner: user.id });
      setTitle("");
    } catch (err) {
      setActionError(err instanceof Error ? err.message : "Couldn't save the note.");
    }
  }

  function confirmDelete(id: string, noteTitle: string) {
    Alert.alert("Delete note?", noteTitle, [
      { text: "Cancel", style: "cancel" },
      {
        text: "Delete",
        style: "destructive",
        onPress: () =>
          remove(id).catch((err: unknown) =>
            setActionError(err instanceof Error ? err.message : "Couldn't delete that note."),
          ),
      },
    ]);
  }

  const failure = error ?? actionError;

  return (
    <KeyboardAvoidingView
      style={[styles.screen, { paddingTop: insets.top + 16, backgroundColor: theme.background }]}
      behavior={Platform.OS === "ios" ? "padding" : undefined}
    >
      <View style={styles.header}>
        <View style={styles.flex}>
          <Text accessibilityRole="header" style={[styles.title, { color: theme.text }]}>
            {"{{PROJECT_NAME}}"}
          </Text>
          <Text numberOfLines={1} style={{ color: theme.muted, fontSize: 13 }}>
            {user?.email}
          </Text>
        </View>
        <Pressable
          accessibilityRole="button"
          hitSlop={8}
          onPress={async () => {
            await signOut();
            router.replace("/sign-in");
          }}
        >
          <Text style={{ color: theme.link, fontWeight: "600" }}>Sign out</Text>
        </Pressable>
      </View>

      <View style={styles.addRow}>
        <TextInput
          accessibilityLabel="New note title"
          style={[styles.input, { borderColor: theme.border, backgroundColor: theme.card, color: theme.text }]}
          placeholder="New note…"
          placeholderTextColor={theme.muted}
          value={title}
          onChangeText={setTitle}
          onSubmitEditing={addNote}
          returnKeyType="done"
        />
        <Pressable
          accessibilityRole="button"
          disabled={pending || !title.trim()}
          style={[styles.addButton, { backgroundColor: theme.accent, opacity: pending || !title.trim() ? 0.5 : 1 }]}
          onPress={addNote}
        >
          <Text style={styles.addButtonText}>Add</Text>
        </Pressable>
      </View>

      {failure != null && (
        <Text accessibilityRole="alert" style={{ color: theme.danger }}>
          {failure instanceof Error ? failure.message : String(failure)}
        </Text>
      )}

      <FlatList
        data={posts}
        keyExtractor={(item) => item.id}
        contentContainerStyle={styles.list}
        keyboardShouldPersistTaps="handled"
        refreshControl={<RefreshControl refreshing={false} onRefresh={refresh} tintColor={theme.muted} />}
        ListEmptyComponent={
          loading ? (
            <ActivityIndicator style={styles.empty} />
          ) : (
            <Text style={[styles.empty, { color: theme.muted }]}>No notes yet — add your first one above.</Text>
          )
        }
        renderItem={({ item }) => (
          <View style={[styles.card, { borderColor: theme.border, backgroundColor: theme.card }]}>
            <Text style={[styles.cardTitle, { color: theme.text }]}>{item.title}</Text>
            <Pressable
              accessibilityRole="button"
              accessibilityLabel={`Delete ${item.title}`}
              hitSlop={8}
              onPress={() => confirmDelete(item.id, item.title)}
            >
              <Text style={{ color: theme.danger, fontSize: 13 }}>Delete</Text>
            </Pressable>
          </View>
        )}
      />
    </KeyboardAvoidingView>
  );
}

const styles = StyleSheet.create({
  flex: { flex: 1 },
  screen: { flex: 1, paddingHorizontal: 20, gap: 16 },
  header: { flexDirection: "row", justifyContent: "space-between", alignItems: "flex-start", gap: 12 },
  title: { fontSize: 20, fontWeight: "700" },
  addRow: { flexDirection: "row", gap: 8 },
  input: { flex: 1, borderWidth: 1, borderRadius: 10, paddingHorizontal: 14, paddingVertical: 10, fontSize: 15 },
  addButton: { borderRadius: 10, paddingHorizontal: 18, justifyContent: "center" },
  addButtonText: { color: "#fff", fontWeight: "600" },
  list: { gap: 10, paddingBottom: 40 },
  card: {
    flexDirection: "row",
    justifyContent: "space-between",
    alignItems: "center",
    gap: 12,
    borderWidth: 1,
    borderRadius: 12,
    padding: 14,
  },
  cardTitle: { fontSize: 15, flexShrink: 1 },
  empty: { textAlign: "center", marginTop: 40 },
});
