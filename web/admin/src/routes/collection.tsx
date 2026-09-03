import { useEffect, useState } from "react";
import { createRoute } from "@tanstack/react-router";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";
import { MoreHorizontal, Plus, ShieldUser } from "lucide-react";
import type { RecordModel } from "cratebase";
import { cb } from "@/lib/api";
import { appRoute } from "@/routes/app";
import { useRecords, useRecordMutations } from "@/hooks/use-records";
import { Tabs } from "@/components/interior/tabs";
import { ExpandingSearch } from "@/components/interior/expanding-search";
import { Pagination } from "@/components/interior/pagination";
import type { SortState } from "@/components/interior/sortable-table";
import { NewItemsPill } from "@/components/interior/new-items-pill";
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuTrigger } from "@/components/ui/dropdown-menu";
import { Button } from "@/components/ui/button";
import { IdCell } from "@/components/records/record-value-cell";
import { InlineEditableCell } from "@/components/records/inline-cell";
import { RecordsGrid, type GridColumn } from "@/components/records/records-grid";
import { RecordDrawer } from "@/components/records/record-drawer";
import { CollectionSettings } from "@/components/collections/collection-settings";

type CollectionSearch = {
  page?: number;
  sort?: string;
  q?: string;
  tab?: "records" | "settings";
};

function parseSortParam(raw: string | undefined): SortState | null {
  if (!raw) return { columnId: "created", direction: "desc" };
  return raw.startsWith("-") ? { columnId: raw.slice(1), direction: "desc" } : { columnId: raw, direction: "asc" };
}

function sortStateToParam(state: SortState | null): string | undefined {
  if (!state) return undefined;
  return state.direction === "desc" ? `-${state.columnId}` : state.columnId;
}

function CollectionPage() {
  const { name } = collectionRoute.useParams();
  const urlSearch = collectionRoute.useSearch();
  const navigate = collectionRoute.useNavigate();
  const queryClient = useQueryClient();

  function updateSearch(patch: Partial<CollectionSearch>) {
    void navigate({ search: (prev) => ({ ...prev, ...patch }), replace: true });
  }

  const tab = urlSearch.tab ?? "records";
  const page = urlSearch.page ?? 1;
  const search = urlSearch.q ?? "";
  const sort = parseSortParam(urlSearch.sort);
  const [editing, setEditing] = useState<RecordModel | null | undefined>(undefined);
  const [newSince, setNewSince] = useState(0);

  const { data: collection } = useQuery({
    queryKey: ["collections", name],
    queryFn: () => cb.collections.getOne(name),
  });

  const sortParam = urlSearch.sort ?? "-created";
  const filter = collection
    ? buildSearchFilter(search, collection.schema.filter((f) => ["text", "email", "url"].includes(f.type)))
    : "";
  const { data: result } = useRecords(name, page, filter, sortParam);
  const { remove } = useRecordMutations(name);


  // Realtime: bump a "new items" pill instead of yanking the list out from
  // under someone mid-read when we're looking at the freshest page.
  useEffect(() => {
    if (!collection) return;
    let unsubscribe: (() => void) | undefined;
    cb.realtime.subscribe(collection.name, (event) => {
      if (event.action === "create" && page === 1 && sort?.columnId === "created" && sort.direction === "desc") {
        setNewSince((n) => n + 1);
      } else {
        void queryClient.invalidateQueries({ queryKey: ["records", name] });
      }
    }).then((fn) => {
      unsubscribe = fn;
    });
    return () => unsubscribe?.();
  }, [collection, name, page, sort, queryClient]);

  if (!collection) {
    return <div className="p-6 text-sm text-muted-foreground">Loading…</div>;
  }

  const columns: GridColumn<RecordModel>[] = [
    {
      id: "id",
      header: "id",
      width: "260px",
      sortable: false,
      cell: (row: RecordModel) => <IdCell id={row.id} />,
    },
    ...(collection.type === "auth"
      ? [
          {
            id: "email",
            header: "email",
            sortable: true,
            value: (row: RecordModel) => String(row.email ?? ""),
            cell: (row: RecordModel) => <span className="text-[13px]">{String(row.email ?? "")}</span>,
          },
        ]
      : []),
    ...collection.schema.map((field) => ({
      id: field.name,
      header: field.name,
      sortable: !["json", "relation", "file"].includes(field.type),
      value: (row: RecordModel) => (typeof row[field.name] === "object" ? JSON.stringify(row[field.name]) : (row[field.name] as string | number)),
      cell: (row: RecordModel) => (
        <InlineEditableCell record={row} field={field} collectionName={collection.name} onOpenDrawer={() => setEditing(row)} />
      ),
    })),
    {
      id: "created",
      header: "created",
      sortable: true,
      value: (row: RecordModel) => row.created,
      cell: (row: RecordModel) => (
        <div
          role="button"
          tabIndex={0}
          onClick={() => setEditing(row)}
          onKeyDown={(e) => e.key === "Enter" && setEditing(row)}
          className="block w-full cursor-pointer text-left font-mono text-[11.5px] text-muted-foreground"
        >
          {new Date(row.created).toLocaleString()}
        </div>
      ),
    },
    {
      id: "__actions",
      header: "",
      width: "40px",
      sortable: false,
      cell: (row: RecordModel) => (
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <Button variant="ghost" size="icon" className="size-7" onClick={(e) => e.stopPropagation()}>
              <MoreHorizontal className="size-4" />
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end" onClick={(e) => e.stopPropagation()}>
            <DropdownMenuItem onClick={() => setEditing(row)}>Edit</DropdownMenuItem>
            <DropdownMenuItem
              variant="destructive"
              onClick={async () => {
                await remove.mutateAsync(row.id);
                toast.success("Record deleted");
              }}
            >
              Delete
            </DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>
      ),
    },
  ];

  return (
    <div className="flex h-full flex-col">
      <header className="flex items-center justify-between border-b border-border px-6 py-4">
        <div className="flex items-center gap-2">
          {collection.type === "auth" ? <ShieldUser className="size-4 text-muted-foreground" /> : null}
          <h1 className="text-[15px] font-semibold tracking-tight">{collection.name}</h1>
        </div>
        <Tabs
          items={[
            { value: "records", label: "Records" },
            { value: "settings", label: "Settings" },
          ]}
          value={tab}
          onValueChange={(next) => updateSearch({ tab: next === "records" ? undefined : (next as "settings") })}
          label="Collection view"
        />
      </header>

      {tab === "settings" ? (
        <div className="flex-1 overflow-y-auto">
          <CollectionSettings collection={collection} />
        </div>
      ) : (
        <>
          <div className="flex items-center justify-between gap-3 px-6 py-3">
            <ExpandingSearch
              value={search}
              onChange={(next) => updateSearch({ q: next || undefined, page: undefined })}
              placeholder={`Search ${collection.name}…`}
              resultCount={result?.totalItems}
            />
            <Button
              size="sm"
              onClick={() => setEditing(null)}
              className="gap-1.5 bg-primary text-primary-foreground hover:bg-primary/90"
            >
              <Plus className="size-3.5" />
              New record
            </Button>
          </div>

          <div className="relative flex-1 overflow-y-auto">
            {newSince > 0 ? (
              <div className="sticky top-11 z-10 flex justify-center">
                <NewItemsPill
                  count={newSince}
                  onJump={() => {
                    setNewSince(0);
                    void queryClient.invalidateQueries({ queryKey: ["records", name] });
                  }}
                />
              </div>
            ) : null}

            {!result ? (
              <div className="px-6">
                <TableSkeleton />
              </div>
            ) : result.items.length > 0 ? (
              <RecordsGrid
                label={`${collection.name} records`}
                rows={result.items}
                columns={columns}
                getRowId={(row: RecordModel) => row.id}
                sort={sort}
                onSortChange={(next) => updateSearch({ sort: sortStateToParam(next), page: undefined })}
              />
            ) : (
              <div className="flex flex-col items-center justify-center gap-2 py-16 text-center">
                <p className="text-sm font-medium">No records yet</p>
                <p className="text-sm text-muted-foreground">Create the first one to see it here.</p>
              </div>
            )}
          </div>

          {result && result.totalPages > 1 ? (
            <div className="flex justify-center border-t border-border py-3">
              <Pagination count={result.totalPages} page={page} onPageChange={(next) => updateSearch({ page: next === 1 ? undefined : next })} label="Records pages" />
            </div>
          ) : null}
        </>
      )}

      {editing !== undefined ? (
        <RecordDrawer collection={collection} record={editing} open={editing !== undefined} onOpenChange={(open) => !open && setEditing(undefined)} />
      ) : null}
    </div>
  );
}

function TableSkeleton() {
  return (
    <div className="flex flex-col gap-2 py-3">
      {Array.from({ length: 6 }).map((_, i) => (
        <div key={i} className="h-9 w-full animate-pulse rounded-lg bg-muted" />
      ))}
    </div>
  );
}

function buildSearchFilter(search: string, textFields: { name: string }[]): string {
  const q = search.trim();
  if (!q || textFields.length === 0) return "";
  const escaped = q.replace(/"/g, '\\"');
  return textFields.map((f) => `${f.name} ~ "${escaped}"`).join(" || ");
}

export const collectionRoute = createRoute({
  getParentRoute: () => appRoute,
  path: "/collections/$name",
  validateSearch: (search: Record<string, unknown>): CollectionSearch => ({
    page: typeof search.page === "number" && search.page > 1 ? search.page : undefined,
    sort: typeof search.sort === "string" ? search.sort : undefined,
    q: typeof search.q === "string" ? search.q : undefined,
    tab: search.tab === "settings" ? "settings" : undefined,
  }),
  component: CollectionPage,
});
