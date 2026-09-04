import { useEffect, useState } from "react";
import { createRoute } from "@tanstack/react-router";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";
import { MoreHorizontal, Plus, Search, Settings as SettingsIcon, ShieldUser } from "lucide-react";
import type { RecordModel } from "pocketbase";
import { cb } from "@/lib/api";
import { userFields } from "@/lib/field-types";
import { appRoute } from "@/routes/app";
import { useRecords, useRecordMutations } from "@/hooks/use-records";
import { Pagination } from "@/components/ui/pagination";
import type { SortState } from "@/hooks/use-sortable-rows";
import { NewItemsPill } from "@/components/ui/new-items-pill";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog";
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuTrigger } from "@/components/ui/dropdown-menu";
import { InputGroup, InputGroupAddon, InputGroupInput, InputGroupText } from "@/components/ui/input-group";
import { Skeleton } from "@/components/ui/skeleton";
import { Button } from "@/components/ui/button";
import { IdCell } from "@/components/records/record-value-cell";
import { InlineEditableCell } from "@/components/records/inline-cell";
import { RecordsGrid, type GridColumn } from "@/components/records/records-grid";
import { RecordDrawer } from "@/components/records/record-drawer";
import { CollectionSettings } from "@/components/collections/collection-settings";

/** Matches the debounce the old expanding-search field committed its
 * query with, so typing still doesn't fire a request per keystroke. */
const SEARCH_DEBOUNCE_MS = 220;

type CollectionSearch = {
  page?: number;
  sort?: string;
  q?: string;
  tab?: "records" | "settings";
  /** Set by a relation-value popover's "Open record" link elsewhere in the
   * dashboard — jumps straight to this collection and pops the record
   * drawer open for the given id, then clears itself from the URL. */
  openId?: string;
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
  const [deleting, setDeleting] = useState<RecordModel | null>(null);
  const [newSince, setNewSince] = useState(0);
  const [searchInput, setSearchInput] = useState(search);

  // A relation-value popover elsewhere in the dashboard links here with
  // `?openId=<id>` instead of a full record — fetch it once, pop the
  // drawer open, then strip the param so a refresh/back-nav doesn't
  // reopen it.
  useEffect(() => {
    const openId = urlSearch.openId;
    if (!openId) return;
    let cancelled = false;
    cb.collection(name)
      .getOne(openId)
      .then((record) => {
        if (!cancelled) setEditing(record);
      })
      .catch(() => {
        if (!cancelled) toast.error("That record no longer exists.");
      })
      .finally(() => {
        if (!cancelled) updateSearch({ openId: undefined });
      });
    return () => {
      cancelled = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [urlSearch.openId, name]);

  // The URL is the source of truth for the query; the field keeps its own
  // state so typing stays responsive and only the committed value lands in
  // the URL (and therefore in the records query).
  useEffect(() => {
    setSearchInput(search);
  }, [search]);

  useEffect(() => {
    if (searchInput === search) return;
    const timer = setTimeout(() => {
      updateSearch({ q: searchInput || undefined, page: undefined });
    }, SEARCH_DEBOUNCE_MS);
    return () => clearTimeout(timer);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [searchInput, search]);

  const { data: collection } = useQuery({
    queryKey: ["collections", name],
    queryFn: () => cb.collections.getOne(name),
  });

  const sortParam = urlSearch.sort ?? "-created";
  const identityField = (collection?.type === "auth" ? collection.passwordAuth?.identityFields?.[0] : undefined) ?? "email";
  const filter = collection
    ? buildSearchFilter(
        search,
        userFields(collection).filter((f) => ["text", "email", "url"].includes(f.type)),
        collection.type === "auth" ? identityField : undefined,
      )
    : "";
  const { data: result } = useRecords(name, page, filter, sortParam);
  const { remove } = useRecordMutations(name);


  // Realtime: bump a "new items" pill instead of yanking the list out from
  // under someone mid-read when we're looking at the freshest page.
  useEffect(() => {
    if (!collection) return;
    let unsubscribe: (() => void) | undefined;
    cb.collection(collection.name).subscribe("*", (event) => {
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
    return (
      <div className="flex flex-col gap-2 p-6">
        {Array.from({ length: 6 }).map((_, i) => (
          <Skeleton key={i} className="h-row w-full" />
        ))}
      </div>
    );
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
            id: identityField,
            header: identityField,
            sortable: true,
            value: (row: RecordModel) => String(row[identityField] ?? ""),
            cell: (row: RecordModel) => <span className="text-sm">{String(row[identityField] ?? "")}</span>,
          },
        ]
      : []),
    ...userFields(collection).map((field) => ({
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
          className="block w-full cursor-pointer text-left font-mono text-xs text-muted-foreground"
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
            <Button variant="ghost" size="icon-sm" onClick={(e) => e.stopPropagation()}>
              <MoreHorizontal className="size-4" />
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end" onClick={(e) => e.stopPropagation()}>
            <DropdownMenuItem onClick={() => setEditing(row)}>Edit</DropdownMenuItem>
            <DropdownMenuItem variant="destructive" onClick={() => setDeleting(row)}>
              Delete
            </DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>
      ),
    },
  ];

  return (
    <div className="flex h-full flex-col">
      <header className="flex items-center justify-between gap-3 border-b border-border px-6 py-4">
        <div className="flex shrink-0 items-center gap-2">
          {collection.type === "auth" ? <ShieldUser className="size-4 text-muted-foreground" /> : null}
          <h1 className="text-base font-semibold tracking-tight">{collection.name}</h1>
          <Button
            variant={tab === "settings" ? "secondary" : "ghost"}
            size="icon-sm"
            aria-label="Collection settings"
            aria-pressed={tab === "settings"}
            onClick={() => updateSearch({ tab: tab === "settings" ? undefined : "settings" })}
          >
            <SettingsIcon className="size-4" />
          </Button>
        </div>

        {tab === "records" ? (
          <div className="flex flex-1 items-center justify-end gap-3">
            <InputGroup className="h-control-sm w-56">
              <InputGroupAddon>
                <Search className="size-3.5" />
              </InputGroupAddon>
              <InputGroupInput
                type="search"
                value={searchInput}
                onChange={(e) => setSearchInput(e.target.value)}
                placeholder={`Search ${collection.name}…`}
                aria-label={`Search ${collection.name}`}
              />
              {searchInput.length > 0 && result ? (
                <InputGroupAddon align="inline-end">
                  <InputGroupText className="font-tabular text-2xs">{result.totalItems}</InputGroupText>
                </InputGroupAddon>
              ) : null}
            </InputGroup>
            <Button size="sm" onClick={() => setEditing(null)} className="gap-1.5">
              <Plus className="size-3.5" />
              New record
            </Button>
          </div>
        ) : (
          <span className="text-sm text-muted-foreground">Settings</span>
        )}
      </header>

      {tab === "settings" ? (
        <div className="flex-1 overflow-y-auto">
          <CollectionSettings collection={collection} />
        </div>
      ) : (
        <>
          <div className="relative flex-1 overflow-y-auto">
            {newSince > 0 ? (
              <div className="sticky top-11 z-raised flex justify-center">
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
              <div className="flex flex-col gap-2 px-6 py-3">
                {Array.from({ length: 6 }).map((_, i) => (
                  <Skeleton key={i} className="h-row w-full" />
                ))}
              </div>
            ) : (
              <RecordsGrid
                label={`${collection.name} records`}
                rows={result.items}
                columns={columns}
                getRowId={(row: RecordModel) => row.id}
                sort={sort}
                onSortChange={(next) => updateSearch({ sort: sortStateToParam(next), page: undefined })}
              />
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

      <AlertDialog open={deleting !== null} onOpenChange={(open) => !open && setDeleting(null)}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Delete this record?</AlertDialogTitle>
            <AlertDialogDescription>
              <span className="font-mono">{deleting?.id}</span> will be permanently removed from{" "}
              <span className="font-mono">{collection.name}</span>. This cannot be undone.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Cancel</AlertDialogCancel>
            <AlertDialogAction
              variant="destructive"
              onClick={async () => {
                const target = deleting;
                setDeleting(null);
                if (!target) return;
                await remove.mutateAsync(target.id);
                toast.success("Record deleted");
              }}
            >
              Delete record
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  );
}

function buildSearchFilter(search: string, textFields: { name: string }[], identityField?: string): string {
  const q = search.trim();
  const names = identityField ? [identityField, ...textFields.map((f) => f.name)] : textFields.map((f) => f.name);
  if (!q || names.length === 0) return "";
  const escaped = q.replace(/"/g, '\\"');
  return names.map((name) => `${name} ~ "${escaped}"`).join(" || ");
}

export const collectionRoute = createRoute({
  getParentRoute: () => appRoute,
  path: "/collections/$name",
  validateSearch: (search: Record<string, unknown>): CollectionSearch => ({
    page: typeof search.page === "number" && search.page > 1 ? search.page : undefined,
    sort: typeof search.sort === "string" ? search.sort : undefined,
    q: typeof search.q === "string" ? search.q : undefined,
    tab: search.tab === "settings" ? "settings" : undefined,
    openId: typeof search.openId === "string" ? search.openId : undefined,
  }),
  component: CollectionPage,
});
