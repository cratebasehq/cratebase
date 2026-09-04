/**
 * Presentation for the server's scheduled jobs.
 *
 * The job ids are PocketBase's verbatim (`__pbLogsCleanup__` and friends)
 * because `GET /api/crons` has to match for tooling written against
 * PocketBase. That is an API compatibility decision, not a naming one —
 * showing a raw internal id as the label tells an operator nothing about
 * what the job does or when it runs, so the id is demoted to secondary
 * detail here and the human description leads.
 */

export interface JobDescription {
  title: string;
  detail: string;
}

const KNOWN: Record<string, JobDescription> = {
  __pbDBOptimize__: {
    title: "Database upkeep",
    detail: "Reclaims space and refreshes query statistics.",
  },
  __pbMFACleanup__: {
    title: "Expired two-factor sessions",
    detail: "Removes half-finished multi-factor logins that were never completed.",
  },
  __pbOTPCleanup__: {
    title: "Expired one-time codes",
    detail: "Removes one-time login codes past their lifetime.",
  },
  __pbLogsCleanup__: {
    title: "Request log trimming",
    detail: "Drops request history older than the retention set on Application.",
  },
  __pbAutoBackup__: {
    title: "Automatic backup",
    detail: "Snapshots the database on the schedule set on Backups.",
  },
};

export function describeJob(id: string): JobDescription {
  return (
    KNOWN[id] ?? {
      title: id,
      detail: "Registered by a plugin or by application code.",
    }
  );
}

const DAYS = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];

/**
 * A five-field cron expression in words. Covers the shapes the server
 * actually registers plus the ones an operator is likely to type; anything
 * more exotic falls back to the expression itself, which is better than a
 * confident wrong reading.
 */
export function describeSchedule(expression: string): string {
  const parts = expression.trim().split(/\s+/);
  if (parts.length !== 5) return expression;
  const [minute, hour, dom, month, dow] = parts;

  const everyDate = dom === "*" && month === "*" && dow === "*";
  // The scheduler ticks on `Utc::now()`, so a wall-clock time here is UTC
  // regardless of where the server or the reader happens to be.
  const at = (h: string, m: string) => `${h.padStart(2, "0")}:${m.padStart(2, "0")} UTC`;

  if (minute === "*" && hour === "*" && everyDate) return "Every minute";

  const stepMinute = minute.match(/^\*\/(\d+)$/);
  if (stepMinute && hour === "*" && everyDate) {
    return `Every ${stepMinute[1]} minutes`;
  }

  const stepHour = hour.match(/^\*\/(\d+)$/);
  if (/^\d+$/.test(minute) && stepHour && everyDate) {
    const past = Number(minute) === 0 ? "on the hour" : `at ${minute} past`;
    return `Every ${stepHour[1]} hours, ${past}`;
  }

  if (/^\d+$/.test(minute) && hour === "*" && everyDate) {
    return Number(minute) === 0 ? "Every hour, on the hour" : `Every hour, at ${minute} past`;
  }

  if (/^\d+$/.test(minute) && /^\d+$/.test(hour)) {
    const time = at(hour, minute);
    if (everyDate) return `Every day at ${time}`;
    if (dom === "*" && month === "*" && /^\d$/.test(dow)) {
      return `Every ${DAYS[Number(dow)]} at ${time}`;
    }
    if (/^\d+$/.test(dom) && month === "*" && dow === "*") {
      return `Day ${dom} of each month at ${time}`;
    }
  }

  return expression;
}
