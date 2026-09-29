import { useState } from "react";
import { FlatList, Pressable, StyleSheet, Text, TextInput, View } from "react-native";
import { Redirect, router } from "expo-router";
import type { RecordModel } from "@cratebase/client";
import { useAuth, useRecords, useMutation } from "@/lib/cratebase";
import type { UsersRecord } from "../cratebase-types";

export default function Notes() {
  const { user, isValid, isLoading, signOut } = useAuth<UsersRecord & RecordModel>();

  // `realtime: true` (the default) refetches this list on every matching
  // create/update/delete event — every open device stays in sync with no
  // manual invalidation. See @cratebase/react's useRecords doc comment.
  const { records: posts, loading } = useRecords("posts", { sort: "-created", realtime: true });
  const { create, remove } = useMutation("posts");
  const [title, setTitle] = useState("");

  if (isLoading) return null;
  if (!isValid) return <Redirect href="/sign-in" />;

  async function addNote() {
    if (!title.trim() || !user) return;
    await create({ title: title.trim(), owner: user.id });
    setTitle("");
  }

  return (
    <View style={styles.screen}>
      <View style={styles.header}>
        <View>
          <Text style={styles.title}>{"{{PROJECT_NAME}}"}</Text>
          <Text style={styles.subtitle}>{user?.email}</Text>
        </View>
        <Pressable
          onPress={async () => {
            await signOut();
            router.replace("/sign-in");
          }}
        >
          <Text style={styles.signOut}>Sign out</Text>
        </Pressable>
      </View>

      <View style={styles.addRow}>
        <TextInput
          style={styles.input}
          placeholder="New note…"
          value={title}
          onChangeText={setTitle}
          onSubmitEditing={addNote}
        />
        <Pressable style={styles.addButton} onPress={addNote}>
          <Text style={styles.addButtonText}>Add</Text>
        </Pressable>
      </View>

      <FlatList
        data={posts}
        keyExtractor={(item) => item.id}
        contentContainerStyle={styles.list}
        ListEmptyComponent={
          loading ? null : <Text style={styles.empty}>No notes yet — add your first one above.</Text>
        }
        renderItem={({ item }) => (
          <View style={styles.card}>
            <Text style={styles.cardTitle}>{item.title}</Text>
            <Pressable onPress={() => remove(item.id)}>
              <Text style={styles.delete}>Delete</Text>
            </Pressable>
          </View>
        )}
      />
    </View>
  );
}

const styles = StyleSheet.create({
  screen: { flex: 1, padding: 20, paddingTop: 60, gap: 16 },
  header: { flexDirection: "row", justifyContent: "space-between", alignItems: "flex-start" },
  title: { fontSize: 20, fontWeight: "700" },
  subtitle: { fontSize: 13, color: "#888" },
  signOut: { color: "#e8622c", fontWeight: "600" },
  addRow: { flexDirection: "row", gap: 8 },
  input: {
    flex: 1,
    borderWidth: 1,
    borderColor: "#ddd",
    borderRadius: 10,
    paddingHorizontal: 14,
    paddingVertical: 10,
    fontSize: 15,
  },
  addButton: { backgroundColor: "#e8622c", borderRadius: 10, paddingHorizontal: 16, justifyContent: "center" },
  addButtonText: { color: "#fff", fontWeight: "600" },
  list: { gap: 10, paddingVertical: 10 },
  card: {
    flexDirection: "row",
    justifyContent: "space-between",
    alignItems: "center",
    borderWidth: 1,
    borderColor: "#eee",
    borderRadius: 12,
    padding: 14,
  },
  cardTitle: { fontSize: 15, flexShrink: 1 },
  delete: { color: "#d33", fontSize: 13 },
  empty: { textAlign: "center", color: "#888", marginTop: 40 },
});
