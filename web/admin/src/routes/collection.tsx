import { useCallback, useEffect, useMemo, useState } from "react";
import { createRoute } from "@tanstack/react-router";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";
import { MoreHorizontal, Plus, Search, Settings as SettingsIcon, ShieldUser, X } from "lucide-react";
import { ClientResponseError, type CollectionModel, type RecordModel } from "pocketbase";
import { avatarUrl, cb, describeFailure } from "@/lib/api";
import { userFields, type FieldSchema } from "@/lib/field-types";
import { appRoute } from "@/routes/app";
import {
  DEFAULT_PAGE_SIZE,
  PAGE_SIZES,
  useBatchCapability,
  useRecordMutations,
  useRecords,
  type RecordsQuery,
} from "@/hooks/use-records";
import { useCollections } from "@/hooks/use-collections";
import { useColumnPrefs } from "@/hooks/use-column-prefs";
import { useLocalState } from "@/hooks/use-local-state";
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
import { InputGroup, InputGroupAddon, InputGroupInput } from "@/components/ui/input-group";
import { Skeleton } from "@/components/ui/skeleton";
import { Button } from "@/components/ui/button";
import { Avatar, AvatarFallback, AvatarImage } from "@/components/ui/avatar";
import { IdCell } from "@/components/records/record-value-cell";
import { InlineEditableCell } from "@/components/records/inline-cell";
import { RecordsGrid, type GridColumn } from "@/components/records/records-grid";
import type { Density } from "@/lib/grid";
import { ColumnsMenu, SelectionBar, ViewMenu } from "@/components/records/records-toolbar";
import { FilterBar } from "@/components/records/filter-bar";
import { GridFooter } from "@/components/records/grid-footer";
import { RecordDrawer } from "@/components/records/record-drawer";
import { CollectionSettings } from "@/components/collections/collection-settings";

/** Matches the debounce the old expanding-search field committed its
 * query with, so typing still doesn't fire a request per keystroke. */
const SEARCH_DEBOUNCE_MS = 220;

/** Column widths, in px, by field type. Fixed widths are what let the grid
 * virtualize and still keep the header, the rows and the scrollbar in
 * agreement; they are sized for the value each type actually holds. */
const COLUMN_WIDTH: Record<string, number> = {
  id: 190,
  created: 190,
  updated: 190,
  text: 240,
  editor: 260,
  email: 220,
  url: 240,
  number: 120,
  bool: 96,
  date: 190,
  autodate: 190,
  select: 190,
  relation: 210,
  file: 170,
  json: 260,
  password: 150,
};

type CollectionSearch = {
  page?: number;
  perPage?: number;
  sort?: string;
  q?: string;
  filter?: string;
  tab?: "records" | "settings";
  /** Set by a relation-value popover's "Open record" link elsewhere in the
   * dashboard — jumps straight to this collection and pops the record
   * drawer open for the given id, then clears itself from the URL. */
  openId?: string;
};

function isDensity(value: unknown): value is Density {
  return value === "comfortable" || value === "compact";
}

function isBoolean(value: unknown): value is boolean {
  return typeof value === "boolean";
}

/** The `q` box is a convenience over the filter language: it ORs a
 * case-insensitive `~` across every text-ish column. Anything more precise
 * is what the filter bar is for. */
function buildSearchFilter(search: string, textFields: { name: string }[], identityField?: string): string {
  const q = search.trim();
  const names = identityField ? [identityField, ...textFields.map((f) => f.name)] : textFields.map((f) => f.name);
  if (!q || names.length === 0) return "";
  const escaped = q.replace(/"/g, '\\"');
  return names.map((name) => `${name} ~ "${escaped}"`).join(" || ");
}

function combineFilters(...parts: (string | undefined)[]): string {
  const kept = parts.map((part) => part?.trim()).filter((part): part is string => Boolean(part));
  if (kept.length === 0) return "";
  if (kept.length === 1) return kept[0]!;
  return kept.map((part) => `(${part})`).join(" && ");
}

function CollectionPage() {
  const { name } = collectionRoute.useParams();
  const urlSearch = collectionRoute.useSearch();
  const navigate = collectionRoute.useNavigate();
  const queryClient = useQueryClient();

  const updateSearch = useCallback(
    (patch: Partial<CollectionSearch>) => {
      void navigate({ search: (prev) => ({ ...prev, ...patch }), replace: true });
    },
    [navigate],
  );

  const tab = urlSearch.tab ?? "records";
  const page = urlSearch.page ?? 1;
  const perPage = urlSearch.perPage ?? DEFAULT_PAGE_SIZE;
  const search = urlSearch.q ?? "";
  const userFilter = urlSearch.filter ?? "";

  const [density, setDensity] = useLocalState<Density>("cratebase:grid-density", "comfortable", isDensity);
  const [countTotal, setCountTotal] = useLocalState<boolean>("cratebase:grid-count-total", true, isBoolean);

  const [editing, setEditing] = useState<RecordModel | null | undefined>(undefined);
  const [deleting, setDeleting] = useState<RecordModel | null>(null);
  const [bulkDeleting, setBulkDeleting] = useState(false);
  const [selected, setSelected] = useState<Set<string>>(() => new Set());
  const [newSince, setNewSince] = useState(0);
  const [searchInput, setSearchInput] = useState(search);
  const [committedSearch, setCommittedSearch] = useState(search);
  const [selectionCollection, setSelectionCollection] = useState(name);
  // Switching tabs unmounts the schema form, so the same unsaved-changes
  // guard that covers navigation has to cover this too.
  const [settingsDirty, setSettingsDirty] = useState(false);
  const [confirmLeaveSettings, setConfirmLeaveSettings] = useState(false);

  const { data: collection, error: collectionError } = useQuery({
    queryKey: ["collections", name],
    queryFn: () => cb.collections.getOne(name),
  });
  // Already in cache for the sidebar; used here to resolve relation targets
  // for sort keys and value labels without a request of its own.
  const { data: allCollections } = useCollections();

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
  }, [urlSearch.openId, name, updateSearch]);

  // The URL is the source of truth for the query; the field keeps its own
  // state so typing stays responsive and only the committed value lands in
  // the URL (and therefore in the records query). Both of these follow a
  // change from outside — a back navigation, a different collection — and
  // are adjusted during render rather than in an effect, so nothing paints
  // the previous collection's selection or the previous search term.
  if (committedSearch !== search) {
    setCommittedSearch(search);
    setSearchInput(search);
  }
  if (selectionCollection !== name) {
    setSelectionCollection(name);
    setSelected(new Set());
  }

  useEffect(() => {
    if (searchInput === search) return;
    const timer = setTimeout(() => {
      updateSearch({ q: searchInput || undefined, page: undefined });
    }, SEARCH_DEBOUNCE_MS);
    return () => clearTimeout(timer);
  }, [searchInput, search, updateSearch]);

  const fields = useMemo(() => (collection ? userFields(collection) : []), [collection]);
  // `created` is part of PocketBase's own scaffold but not guaranteed: a
  // collection created through the API with an explicit `fields` array may
  // not have it, and sorting by a column that doesn't exist is a 400.
  const hasCreated = collection?.fields.some((f) => f.name === "created") ?? false;
  const sortParam = urlSearch.sort ?? (hasCreated ? "-created" : "-id");
  const identityField =
    (collection?.type === "auth" ? collection.passwordAuth?.identityFields?.[0] : undefined) ?? "email";

  const searchFilter = collection
    ? buildSearchFilter(
        search,
        fields.filter((f) => ["text", "email", "url"].includes(f.type)),
        collection.type === "auth" ? identityField : undefined,
      )
    : "";

  // One request, not one per relation cell: ask the server to inline every
  // relation this grid renders.
  const expand = fields
    .filter((f) => f.type === "relation")
    .map((f) => f.name)
    .join(",");

  const query: RecordsQuery = {
    page,
    perPage,
    filter: combineFilters(searchFilter, userFilter),
    sort: sortParam,
    expand,
    skipTotal: !countTotal,
  };

  const records = useRecords(collection ? name : "", query);
  const { remove, removeMany } = useRecordMutations(name);
  const batch = useBatchCapability();

  const result = records.data;
  const rows = result?.items ?? [];

  // A 400 with a filter in play is almost always the filter — show the
  // server's own sentence under the expression rather than as a toast that
  // scrolls away.
  const failure = records.error ? describeFailure(records.error) : null;
  const filterError =
    failure && failure.status === 400 && (userFilter.length > 0 || search.length > 0)
      ? failure.serverMessage || failure.detail || "The server rejected this filter."
      : null;

  // Realtime: bump a "new items" pill instead of yanking the list out from
  // under someone mid-read when we're looking at the freshest page.
  useEffect(() => {
    if (!collection) return;
    let unsubscribe: (() => void) | undefined;
    void cb
      .collection(collection.name)
      .subscribe("*", (event) => {
        if (event.action === "create" && page === 1 && sortParam === "-created") {
          setNewSince((n) => n + 1);
        } else {
          void queryClient.invalidateQueries({ queryKey: ["records", name] });
        }
      })
      .then((fn) => {
        unsubscribe = fn;
      })
      // A server without the realtime endpoint is a perfectly usable
      // server — the grid just doesn't get its live "new items" pill.
      // Failing loudly here would put a rejection in the console on every
      // page view for no action anyone can take.
      .catch(() => undefined);
    return () => unsubscribe?.();
  }, [collection, name, page, sortParam, queryClient]);

  /* ---- Columns -------------------------------------------------------- */

  const allColumnIds = useMemo(() => {
    if (!collection) return [];
    return [
      "id",
      ...(collection.type === "auth" ? [identityField] : []),
      ...fields.map((f) => f.name),
      ...(hasCreated ? ["created"] : []),
    ];
  }, [collection, fields, identityField, hasCreated]);

  const columnPrefs = useColumnPrefs(name, allColumnIds, ["id"]);

  const columnLabels = useMemo(() => {
    const labels: Record<string, string> = { id: "id", created: "created" };
    for (const field of fields) labels[field.name] = field.name;
    if (collection?.type === "auth") labels[identityField] = identityField;
    return labels;
  }, [fields, collection, identityField]);

  const openDrawer = useCallback((record: RecordModel) => setEditing(record), []);

  const columns = useMemo<GridColumn<RecordModel>[]>(() => {
    if (!collection) return [];
    const byId = new Map<string, GridColumn<RecordModel>>();

    byId.set("id", {
      id: "id",
      header: "id",
      width: COLUMN_WIDTH.id!,
      sortKey: "id",
      cell: (row) => (
        <div
          className="flex min-w-0 flex-1 items-center"
          onDoubleClick={() => openDrawer(row)}
          title="Double-click, or press Enter, to open this record"
        >
          <IdCell id={row.id} />
        </div>
      ),
    });

    if (collection.type === "auth") {
      const avatarField = fields.find((f) => f.type === "file" && f.name === "avatar");
      byId.set(identityField, {
        id: identityField,
        header: identityField,
        width: COLUMN_WIDTH.email!,
        sortKey: identityField,
        cell: (row) => {
          const filename = avatarField ? (row[avatarField.name] as string | undefined) : undefined;
          const avatarSrc = filename ? cb.files.getURL(row, filename) : avatarUrl(row.id);
          return (
            <div className="flex min-w-0 items-center gap-2">
              <Avatar className="size-5 shrink-0 rounded-full">
                <AvatarImage src={avatarSrc} alt="" />
                <AvatarFallback className="text-2xs" />
              </Avatar>
              <span className="truncate text-sm">{String(row[identityField] ?? "")}</span>
            </div>
          );
        },
      });
    }

    for (const field of fields) {
      byId.set(field.name, {
        id: field.name,
        header: field.name,
        width: COLUMN_WIDTH[field.type] ?? COLUMN_WIDTH.text!,
        sortKey: sortKeyFor(field, allCollections),
        editable: true,
        cell: (row, state) => (
          <InlineEditableCell
            record={row}
            field={field}
            collectionName={collection.name}
            editing={state.editing}
            onBeginEdit={state.beginEdit}
            onEndEdit={state.endEdit}
            onOpenDrawer={() => openDrawer(row)}
          />
        ),
      });
    }

    if (hasCreated) {
      byId.set("created", {
        id: "created",
        header: "created",
        width: COLUMN_WIDTH.created!,
        sortKey: "created",
        cell: (row) => (
          <span
            className="min-w-0 flex-1 truncate font-mono text-xs text-muted-foreground"
            onDoubleClick={() => openDrawer(row)}
            title="Double-click, or press Enter, to open this record"
          >
            {new Date(row.created).toLocaleString()}
          </span>
        ),
      });
    }

    return columnPrefs.visible.map((id) => byId.get(id)).filter((c): c is GridColumn<RecordModel> => Boolean(c));
  }, [collection, fields, identityField, columnPrefs.visible, openDrawer, allCollections, hasCreated]);

  /* ---- Loading / not found -------------------------------------------- */

  if (collectionError) {
    const failed = describeFailure(collectionError);
    return (
      <div className="flex h-full flex-col items-center justify-center gap-2 p-page text-center">
        <p className="text-lg font-medium">{failed.title}</p>
        <p className="max-w-measure text-sm text-muted-foreground">{failed.detail || failed.serverMessage}</p>
      </div>
    );
  }

  if (!collection) {
    return (
      <div className="flex flex-col gap-2 p-page">
        {Array.from({ length: 8 }).map((_, i) => (
          <Skeleton key={i} className="h-row w-full" />
        ))}
      </div>
    );
  }

  const totalKnown = (result?.totalItems ?? -1) >= 0;
  const status = records.isError ? "error" : records.isPending ? "loading" : "ready";

  return (
    <div className="flex h-full min-h-0 flex-col">
      <header className="flex h-topbar shrink-0 items-center justify-between gap-3 border-b border-border px-page">
        <div className="flex min-w-0 shrink-0 items-center gap-2">
          {collection.type === "auth" ? <ShieldUser className="size-4 shrink-0 text-muted-foreground" /> : null}
          <h1 className="truncate text-base font-semibold tracking-tight">{collection.name}</h1>
          {tab === "records" ? (
            <span className="shrink-0 font-tabular text-xs text-muted-foreground">
              {totalKnown ? `${result!.totalItems.toLocaleString()} rows` : `page ${page}`}
            </span>
          ) : null}
        </div>

        <div className="flex flex-1 items-center justify-end gap-2">
          {tab === "records" ? (
            <>
              <InputGroup className="h-control-sm w-48">
                <InputGroupAddon>
                  <Search className="size-3.5" />
                </InputGroupAddon>
                <InputGroupInput
                  type="search"
                  value={searchInput}
                  onChange={(e) => setSearchInput(e.target.value)}
                  placeholder="Search…"
                  aria-label={`Search ${collection.name}`}
                />
              </InputGroup>
              <Button size="sm" className="h-control-sm gap-1.5" onClick={() => setEditing(null)}>
                <Plus className="size-3.5" />
                New record
              </Button>
            </>
          ) : null}
          <Button
            variant={tab === "settings" ? "secondary" : "ghost"}
            size="icon-sm"
            aria-label="Collection settings"
            aria-pressed={tab === "settings"}
            onClick={() => {
              if (tab === "settings" && settingsDirty) {
                setConfirmLeaveSettings(true);
                return;
              }
              updateSearch({ tab: tab === "settings" ? undefined : "settings" });
            }}
          >
            <SettingsIcon className="size-4" />
          </Button>
        </div>
      </header>

      {tab === "settings" ? (
        <div className="min-h-0 flex-1 overflow-y-auto">
          <CollectionSettings collection={collection} onDirtyChange={setSettingsDirty} />
        </div>
      ) : (
        <>
          <div className="flex shrink-0 items-start gap-2 border-b border-border px-page py-1.5">
            <FilterBar
              className="min-w-0 flex-1"
              value={userFilter}
              onApply={(next) => updateSearch({ filter: next || undefined, page: undefined })}
              error={filterError}
              collectionName={collection.name}
              fields={fields}
            />
            <div className="flex shrink-0 items-center gap-2">
              <ColumnsMenu prefs={columnPrefs} labels={columnLabels} locked={["id"]} />
              <ViewMenu
                perPage={perPage}
                onPerPageChange={(next) =>
                  updateSearch({ perPage: next === DEFAULT_PAGE_SIZE ? undefined : next, page: undefined })
                }
                density={density}
                onDensityChange={setDensity}
                countTotal={countTotal}
                onCountTotalChange={setCountTotal}
              />
            </div>
          </div>

          <SelectionBar
            count={selected.size}
            onClear={() => setSelected(new Set())}
            onDelete={() => setBulkDeleting(true)}
            batchEnabled={batch.data?.enabled !== false}
            busy={removeMany.isPending}
          />

          <div className="relative min-h-0 flex-1">
            {newSince > 0 ? (
              <div className="pointer-events-none absolute inset-x-0 top-9 z-raised flex justify-center">
                <div className="pointer-events-auto">
                  <NewItemsPill
                    count={newSince}
                    onJump={() => {
                      setNewSince(0);
                      void queryClient.invalidateQueries({ queryKey: ["records", name] });
                    }}
                  />
                </div>
              </div>
            ) : null}

            <RecordsGrid
              label={`${collection.name} records`}
              rows={rows}
              columns={columns}
              getRowId={(row) => row.id}
              sort={sortParam}
              onSortChange={(next) => updateSearch({ sort: next || undefined, page: undefined })}
              selected={selected}
              onSelectedChange={setSelected}
              onOpenRow={openDrawer}
              status={status}
              error={
                failure
                  ? {
                      // When the filter bar is already showing the server's
                      // sentence, repeating it here says nothing — point at
                      // the thing to change instead.
                      title: filterError ? "That filter didn't parse" : failure.title,
                      detail: filterError
                        ? "Fix the expression above and press Enter. The ? next to it lists every operator, macro and modifier the server understands."
                        : failure.serverMessage || failure.detail,
                    }
                  : null
              }
              onRetry={() => void records.refetch()}
              refreshing={records.isFetching && !records.isPending}
              density={density}
              emptyTitle={
                userFilter || search ? "Nothing matches that query" : `No records in ${collection.name} yet`
              }
              emptyDescription={
                userFilter || search
                  ? "Loosen the filter, or clear it to see the whole collection."
                  : "Rows added here — or written through the API — show up in this grid."
              }
              emptyAction={
                userFilter || search ? (
                  <Button
                    size="sm"
                    variant="outline"
                    className="gap-1.5"
                    onClick={() => updateSearch({ filter: undefined, q: undefined, page: undefined })}
                  >
                    <X className="size-3.5" />
                    Clear filters
                  </Button>
                ) : (
                  <Button size="sm" className="gap-1.5" onClick={() => setEditing(null)}>
                    <Plus className="size-3.5" />
                    New record
                  </Button>
                )
              }
              rowActions={(row) => (
                <DropdownMenu>
                  <DropdownMenuTrigger asChild>
                    <Button
                      variant="ghost"
                      size="icon-sm"
                      aria-label={`Actions for ${row.id}`}
                      className="size-control-xs opacity-0 group-hover/row:opacity-100 data-[state=open]:opacity-100"
                    >
                      <MoreHorizontal className="size-3.5" />
                    </Button>
                  </DropdownMenuTrigger>
                  <DropdownMenuContent align="end">
                    <DropdownMenuItem onClick={() => setEditing(row)}>Edit record</DropdownMenuItem>
                    <DropdownMenuItem variant="destructive" onClick={() => setDeleting(row)}>
                      Delete record
                    </DropdownMenuItem>
                  </DropdownMenuContent>
                </DropdownMenu>
              )}
            />
          </div>

          <GridFooter
            page={page}
            perPage={perPage}
            rowsOnPage={rows.length}
            totalItems={result?.totalItems ?? -1}
            totalPages={result?.totalPages ?? -1}
            onPageChange={(next) => updateSearch({ page: next === 1 ? undefined : next })}
          />
        </>
      )}

      {editing !== undefined ? (
        <RecordDrawer
          collection={collection}
          record={editing}
          open={editing !== undefined}
          onOpenChange={(open) => !open && setEditing(undefined)}
        />
      ) : null}

      <AlertDialog open={deleting !== null} onOpenChange={(open) => !open && setDeleting(null)}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Delete this record?</AlertDialogTitle>
            <AlertDialogDescription>
              <span className="font-mono">{deleting?.id}</span> will be permanently removed from{" "}
              <span className="font-mono">{collection.name}</span>, along with any files attached to it. This cannot
              be undone.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Cancel</AlertDialogCancel>
            <AlertDialogAction
              variant="destructive"
              onClick={() => {
                const target = deleting;
                setDeleting(null);
                if (!target) return;
                remove.mutate(target.id, {
                  onSuccess: () => toast.success("Record deleted"),
                  onError: (error) => {
                    const failed = describeFailure(error);
                    toast.error(failed.title, { description: failed.detail || undefined });
                  },
                });
              }}
            >
              Delete record
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>

      <AlertDialog open={confirmLeaveSettings} onOpenChange={(open) => !open && setConfirmLeaveSettings(false)}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Leave the schema editor?</AlertDialogTitle>
            <AlertDialogDescription>
              You have unsaved changes to <span className="font-mono">{collection.name}</span>'s schema. Going back to
              the records view loses them.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Keep editing</AlertDialogCancel>
            <AlertDialogAction
              variant="destructive"
              onClick={() => {
                setConfirmLeaveSettings(false);
                setSettingsDirty(false);
                updateSearch({ tab: undefined });
              }}
            >
              Leave and lose them
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>

      <AlertDialog open={bulkDeleting} onOpenChange={(open) => !open && setBulkDeleting(false)}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>
              Delete {selected.size.toLocaleString()} {selected.size === 1 ? "record" : "records"}?
            </AlertDialogTitle>
            <AlertDialogDescription>
              They will be permanently removed from <span className="font-mono">{collection.name}</span> in a single
              transactional <span className="font-mono">POST /api/batch</span> — all of them, or none. This cannot be
              undone.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Cancel</AlertDialogCancel>
            <AlertDialogAction
              variant="destructive"
              onClick={() => {
                const ids = [...selected];
                setBulkDeleting(false);
                removeMany.mutate(
                  { ids, chunkSize: batch.data?.maxRequests ?? 50 },
                  {
                    onSuccess: () => {
                      setSelected(new Set());
                      toast.success(`Deleted ${ids.length.toLocaleString()} records`);
                    },
                    onError: (error) => {
                      const failed = describeFailure(error);
                      toast.error(failed.title, {
                        description:
                          error instanceof ClientResponseError && error.status === 403
                            ? "The batch API is disabled — enable it in Settings to delete in bulk."
                            : failed.serverMessage || failed.detail || undefined,
                      });
                    },
                  },
                );
              }}
            >
              Delete {selected.size.toLocaleString()}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  );
}

/** Sorting a relation column by its stored id is never what anyone means.
 * The server sorts *through* relations (`author.name`), so point the header
 * at the target collection's display field when there is one. `json` and
 * `file` columns hold documents and filenames — the server rejects those. */
function sortKeyFor(field: FieldSchema, collections: CollectionModel[] | undefined): string | undefined {
  if (field.type === "json" || field.type === "file") return undefined;
  if (field.type === "relation") {
    const target = collections?.find((c) => c.id === field.collectionId);
    const display = target ? userFields(target).find((f) => f.type === "text")?.name : undefined;
    return display ? `${field.name}.${display}` : undefined;
  }
  return field.name;
}

export const collectionRoute = createRoute({
  getParentRoute: () => appRoute,
  path: "/collections/$name",
  validateSearch: (search: Record<string, unknown>): CollectionSearch => ({
    page: typeof search.page === "number" && search.page > 1 ? search.page : undefined,
    perPage:
      typeof search.perPage === "number" && (PAGE_SIZES as readonly number[]).includes(search.perPage)
        ? search.perPage
        : undefined,
    sort: typeof search.sort === "string" ? search.sort : undefined,
    q: typeof search.q === "string" ? search.q : undefined,
    filter: typeof search.filter === "string" ? search.filter : undefined,
    tab: search.tab === "settings" ? "settings" : undefined,
    openId: typeof search.openId === "string" ? search.openId : undefined,
  }),
  component: CollectionPage,
});
