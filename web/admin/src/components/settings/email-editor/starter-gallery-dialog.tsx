import { useRef, type CSSProperties, type KeyboardEvent } from "react";
import { Link } from "@tanstack/react-router";
import { ArrowLeft } from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { STARTER_TEMPLATES, type StarterTemplate } from "./starter-templates";

/**
 * Shown once, right when opening `/settings/email-templates/new` —
 * "Blank" starts an empty document, the other eight seed the visual
 * editor with a ready-made document (see `starter-templates.ts`) for
 * the author to customize rather than stare at a blank canvas.
 *
 * Arrow keys move between cards (Up/Down jump a whole row); Esc starts
 * from Blank rather than trapping the author in the dialog.
 */
export function StarterGalleryDialog({
  open,
  onPick,
}: {
  open: boolean;
  onPick: (template: StarterTemplate) => void;
}) {
  const gridRef = useRef<HTMLDivElement>(null);

  function onGridKeyDown(event: KeyboardEvent<HTMLDivElement>) {
    const keys = ["ArrowRight", "ArrowLeft", "ArrowDown", "ArrowUp", "Home", "End"];
    if (!keys.includes(event.key)) return;
    const cards = Array.from(gridRef.current?.querySelectorAll<HTMLButtonElement>("button") ?? []);
    const index = cards.findIndex((card) => card === document.activeElement);
    if (index < 0 || cards.length === 0) return;
    // Columns = cards sharing the first card's row.
    const columns = cards.filter((card) => card.offsetTop === cards[0].offsetTop).length || 1;
    let next = index;
    if (event.key === "ArrowRight") next = index + 1;
    else if (event.key === "ArrowLeft") next = index - 1;
    else if (event.key === "ArrowDown") next = index + columns;
    else if (event.key === "ArrowUp") next = index - columns;
    else if (event.key === "Home") next = 0;
    else next = cards.length - 1;
    if (next < 0 || next >= cards.length) return;
    event.preventDefault();
    cards[next].focus();
  }

  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        if (next) return;
        const blank = STARTER_TEMPLATES.find((template) => template.id === "blank");
        if (blank) onPick(blank);
      }}
    >
      <DialogContent className="sm:max-w-2xl" showCloseButton={false} onPointerDownOutside={(e) => e.preventDefault()}>
        <DialogHeader>
          <DialogTitle>Start from a template</DialogTitle>
          <DialogDescription>
            Pick a starting point for the visual editor — you can change everything afterward. Esc starts blank.
          </DialogDescription>
        </DialogHeader>
        <div
          ref={gridRef}
          role="group"
          aria-label="Starter templates"
          onKeyDown={onGridKeyDown}
          className="grid max-h-[60vh] grid-cols-2 gap-2 overflow-y-auto p-0.5 sm:grid-cols-3"
        >
          {STARTER_TEMPLATES.map((template, index) => {
            const Icon = template.icon;
            return (
              <button
                key={template.id}
                type="button"
                autoFocus={index === 0}
                onClick={() => onPick(template)}
                style={{ "--i": index } as CSSProperties}
                className="animate-rise flex flex-col items-start gap-2 rounded-lg border border-border bg-card p-3 text-left shadow-xs transition-[border-color,box-shadow] duration-fast ease-out-strong hover:border-border-strong hover:shadow-e2 focus-visible:border-ring focus-visible:ring-3 focus-visible:ring-ring/40"
              >
                <span
                  aria-hidden
                  className="flex size-control-md items-center justify-center rounded-md bg-surface-sunken text-muted-foreground [&_svg]:size-4"
                >
                  <Icon />
                </span>
                <span className="flex flex-col gap-0.5">
                  <span className="text-sm font-medium">{template.name}</span>
                  <span className="text-xs text-pretty text-muted-foreground">{template.description}</span>
                </span>
              </button>
            );
          })}
        </div>
        <DialogFooter className="sm:justify-start">
          <Button variant="ghost" size="sm" asChild>
            <Link to="/settings/email" search={{ tab: "templates" }}>
              <ArrowLeft />
              Back to templates
            </Link>
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
