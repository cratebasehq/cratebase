import { useState } from "react";
import { useNavigate } from "react-router-dom";
import type { RecordModel } from "@cratebase/client";
import { useAuth, useRecords } from "@/lib/cratebase";
import type { UsersRecord } from "../../cratebase-types";
import { NotificationsBell } from "@/components/NotificationsBell";
import { SearchBox } from "@/components/SearchBox";
import { PostForm } from "@/components/PostForm";
import { PostList } from "@/components/PostList";

export function Dashboard() {
  const { user, signOut } = useAuth<UsersRecord & RecordModel>();
  const navigate = useNavigate();
  const [search, setSearch] = useState("");

  // `realtime: true` (the default) refetches this list on every matching
  // create/update/delete event — every open tab stays in sync with no
  // manual invalidation. `search` runs Cratebase's full-text index over
  // `posts`' searchable fields (title, content), ANDed onto the owner
  // filter the collection's own listRule already applies server-side.
  const { records: posts, loading } = useRecords("posts", {
    sort: "-created",
    search: search.trim() || undefined,
    realtime: true,
  });

  return (
    <div className="mx-auto max-w-2xl px-6 py-10">
      <header className="mb-8 flex items-center justify-between gap-4">
        <div>
          <h1 className="text-lg font-semibold">{"{{PROJECT_NAME}}"}</h1>
          <p className="text-sm text-black/50 dark:text-white/50">Signed in as {user?.email}</p>
        </div>
        <div className="flex items-center gap-2">
          <NotificationsBell />
          <button
            type="button"
            className="btn-secondary"
            onClick={async () => {
              await signOut();
              navigate("/");
            }}
          >
            Sign out
          </button>
        </div>
      </header>

      <div className="mb-6">
        <SearchBox value={search} onChange={setSearch} />
      </div>

      <div className="mb-6">
        <PostForm />
      </div>

      <PostList posts={posts} loading={loading} />
    </div>
  );
}
