import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { STARTER_TEMPLATES, type StarterTemplate } from "./starter-templates";

/**
 * Shown once, right when opening `/settings/email-templates/new` —
 * "Blank" starts an empty document, the other eight seed the visual
 * editor with a ready-made document (see `starter-templates.ts`) for
 * the author to customize rather than stare at a blank canvas.
 */
export function StarterGalleryDialog({
  open,
  onPick,
}: {
  open: boolean;
  onPick: (template: StarterTemplate) => void;
}) {
  return (
    <Dialog open={open}>
      <DialogContent
        className="sm:max-w-2xl"
        showCloseButton={false}
        onEscapeKeyDown={(e) => e.preventDefault()}
        onPointerDownOutside={(e) => e.preventDefault()}
      >
        <DialogHeader>
          <DialogTitle>Start from a template</DialogTitle>
          <DialogDescription>
            Pick a starting point for the visual editor — you can change everything afterward.
          </DialogDescription>
        </DialogHeader>
        <div className="grid grid-cols-2 gap-2 sm:grid-cols-3">
          {STARTER_TEMPLATES.map((template) => {
            const Icon = template.icon;
            return (
              <button
                key={template.id}
                type="button"
                onClick={() => onPick(template)}
                className="flex flex-col items-start gap-1.5 rounded-lg border border-border p-3 text-left transition-colors hover:border-primary hover:bg-accent"
              >
                <Icon className="size-4 text-muted-foreground" />
                <span className="text-sm font-medium">{template.name}</span>
                <span className="text-xs text-muted-foreground">{template.description}</span>
              </button>
            );
          })}
        </div>
      </DialogContent>
    </Dialog>
  );
}
