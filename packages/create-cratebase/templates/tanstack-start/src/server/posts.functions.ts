import { createServerFn } from "@tanstack/react-start";
import { cb } from "~/lib/cratebase";

export interface Note {
  id: string;
  title: string;
  content: string;
  created: string;
}

interface PostRow {
  id: string;
  title: string;
  content?: string;
  created?: string;
}

const toNote = (row: PostRow): Note => ({
  id: row.id,
  title: row.title,
  content: row.content ?? "",
  created: row.created ?? "",
});

/** Public feed: the latest notes from everyone (the `posts` list rule is open in this template's schema.json). */
export const listPublicNotesFn = createServerFn({ method: "GET" }).handler(async (): Promise<Note[]> => {
  const { items } = await cb.client().collection("posts").list({ sort: "-created", perPage: 12, skipTotal: true });
  return items.map(toNote);
});

/** The signed-in user's own notes. */
export const listMyNotesFn = createServerFn({ method: "GET" })
  .middleware([cb.authMiddleware])
  .handler(async ({ context }): Promise<Note[]> => {
    const { items } = await cb
      .client()
      .collection("posts")
      .list({ filter: `owner = "${context.user.id}"`, sort: "-created", perPage: 50, skipTotal: true });
    return items.map(toNote);
  });

export const createNoteFn = createServerFn({ method: "POST" })
  .middleware([cb.authMiddleware])
  .validator((data: unknown) => {
    const { title, content } = (data ?? {}) as { title?: unknown; content?: unknown };
    if (typeof title !== "string" || title.trim() === "") throw new Error("A title is required.");
    return { title: title.trim(), content: typeof content === "string" ? content : "" };
  })
  .handler(async ({ data, context }): Promise<Note> => {
    const row = await cb.client().collection("posts").create({ ...data, owner: context.user.id });
    return toNote(row);
  });

export const deleteNoteFn = createServerFn({ method: "POST" })
  .middleware([cb.authMiddleware])
  .validator((data: unknown) => {
    const { id } = (data ?? {}) as { id?: unknown };
    if (typeof id !== "string" || id === "") throw new Error("Missing note id.");
    return { id };
  })
  .handler(async ({ data }) => {
    // The collection's delete rule (`owner = @request.auth.id`) is the real authorization check.
    await cb.client().collection("posts").delete(data.id);
    return { ok: true as const };
  });
