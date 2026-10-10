import { useId, useState, type ReactNode } from "react";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Spinner } from "@/components/ui/spinner";

/**
 * Destructive confirmation that makes the operator type the resource name
 * before the action unlocks. Enter submits once the name matches; Esc and
 * Cancel close and return focus to the trigger (Radix handles that).
 * Mount it with `target` set to open it, `null` to close — the typed text
 * resets every time it reopens.
 */
export function TypedConfirmDialog({
  open,
  onOpenChange,
  title,
  description,
  confirmValue,
  confirmLabel,
  pendingLabel,
  pending = false,
  onConfirm,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  title: string;
  description: ReactNode;
  /** The exact text the operator must type. */
  confirmValue: string;
  confirmLabel: string;
  pendingLabel?: string;
  pending?: boolean;
  onConfirm: () => void;
}) {
  const [typed, setTyped] = useState("");
  const [seedOpen, setSeedOpen] = useState(open);
  const inputId = useId();
  if (seedOpen !== open) {
    setSeedOpen(open);
    if (open) setTyped("");
  }
  const matches = typed === confirmValue;

  return (
    <Dialog open={open} onOpenChange={(next) => (!pending || next) && onOpenChange(next)}>
      <DialogContent className="sm:max-w-md">
        <form
          noValidate
          className="grid gap-4"
          onSubmit={(event) => {
            event.preventDefault();
            if (matches && !pending) onConfirm();
          }}
        >
          <DialogHeader>
            <DialogTitle>{title}</DialogTitle>
            <DialogDescription>{description}</DialogDescription>
          </DialogHeader>

          <div className="flex flex-col gap-1.5">
            <Label htmlFor={inputId}>
              Type <span className="font-mono font-semibold text-foreground">{confirmValue}</span> to confirm
            </Label>
            <Input
              id={inputId}
              value={typed}
              onChange={(event) => setTyped(event.target.value)}
              autoComplete="off"
              spellCheck={false}
              autoFocus
              className="h-control-md font-mono"
              placeholder={confirmValue}
            />
          </div>

          <DialogFooter>
            <Button type="button" variant="outline" onClick={() => onOpenChange(false)} disabled={pending}>
              Cancel
            </Button>
            <Button type="submit" variant="destructive" disabled={!matches || pending}>
              {pending ? <Spinner className="size-3.5" /> : null}
              {pending ? (pendingLabel ?? confirmLabel) : confirmLabel}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
