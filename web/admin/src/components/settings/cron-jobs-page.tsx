import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Clock, Play } from "lucide-react";
import { toast } from "sonner";
import { cb } from "@/lib/api";
import { describeFailure } from "@/lib/api";
import { Button } from "@/components/ui/button";
import { Empty, EmptyDescription, EmptyHeader, EmptyMedia, EmptyTitle } from "@/components/ui/empty";
import { Skeleton } from "@/components/ui/skeleton";
import { Spinner } from "@/components/ui/spinner";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";

/** As `GET /api/crons` returns them. */
type CronJob = {
  id: string;
  expression: string;
};

/**
 * Jobs the server has registered, and a way to run one now.
 *
 * These are registered in code, not stored as records — there is no
 * user-editable cron collection — so this screen lists and triggers
 * rather than offering CRUD.
 */
export function CronJobsPage() {
  const queryClient = useQueryClient();

  const { data, isLoading, error } = useQuery({
    queryKey: ["crons"],
    queryFn: () => cb.send<CronJob[]>("/api/crons", { method: "GET" }),
  });

  const run = useMutation({
    mutationFn: (id: string) =>
      cb.send<void>(`/api/crons/${encodeURIComponent(id)}`, { method: "POST" }),
    onSuccess: (_result, id) => {
      toast.success(`Ran ${id}`);
      // A job that touches the log or the database changes what the other
      // settings screens show.
      void queryClient.invalidateQueries({ queryKey: ["request-logs"] });
      void queryClient.invalidateQueries({ queryKey: ["backups"] });
    },
    onError: (failure) => {
      const described = describeFailure(failure);
      toast.error(described.title, { description: described.detail });
    },
  });

  const jobs = data ?? [];

  return (
    <div className="flex flex-col gap-page">
      <p className="max-w-measure text-sm text-muted-foreground">
        Scheduled work the server runs on its own — log trimming, expired
        one-time codes, database upkeep, and the automatic backup once one is
        configured. Running a job here executes it immediately without
        affecting its schedule.
      </p>

      {error ? (
        <Empty>
          <EmptyHeader>
            <EmptyMedia variant="icon">
              <Clock />
            </EmptyMedia>
            <EmptyTitle>Couldn't load the schedule</EmptyTitle>
            <EmptyDescription>{describeFailure(error).detail}</EmptyDescription>
          </EmptyHeader>
        </Empty>
      ) : isLoading ? (
        <div className="flex flex-col gap-2">
          {Array.from({ length: 4 }, (_, i) => (
            <Skeleton key={i} className="h-row w-full" />
          ))}
        </div>
      ) : jobs.length === 0 ? (
        <Empty>
          <EmptyHeader>
            <EmptyMedia variant="icon">
              <Clock />
            </EmptyMedia>
            <EmptyTitle>No scheduled jobs</EmptyTitle>
            <EmptyDescription>Nothing is registered on this server.</EmptyDescription>
          </EmptyHeader>
        </Empty>
      ) : (
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead>Job</TableHead>
              <TableHead>Schedule</TableHead>
              <TableHead className="w-24 text-right">Run</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {jobs.map((job) => (
              <TableRow key={job.id}>
                <TableCell className="font-mono text-xs">{job.id}</TableCell>
                <TableCell className="font-mono text-xs text-muted-foreground">
                  {job.expression}
                </TableCell>
                <TableCell className="text-right">
                  <Button
                    variant="ghost"
                    size="sm"
                    aria-label={`Run ${job.id}`}
                    disabled={run.isPending && run.variables === job.id}
                    onClick={() => run.mutate(job.id)}
                  >
                    {run.isPending && run.variables === job.id ? (
                      <Spinner className="size-3.5" />
                    ) : (
                      <Play className="size-3.5" />
                    )}
                    Run now
                  </Button>
                </TableCell>
              </TableRow>
            ))}
          </TableBody>
        </Table>
      )}
    </div>
  );
}
