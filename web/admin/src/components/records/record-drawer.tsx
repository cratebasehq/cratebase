import { useEffect, useState } from "react";
import { toast } from "sonner";
import type { CollectionModel, RecordModel } from "pocketbase";
import { isMultiValue, userFields } from "@/lib/field-types";
import {
  Sheet,
  SheetContent,
  SheetDescription,
  SheetFooter,
  SheetHeader,
  SheetTitle,
} from "@/components/ui/sheet";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Progress } from "@/components/ui/progress";
import { Spinner } from "@/components/ui/spinner";
import { RecordFieldInput, existingRecordValue } from "@/components/records/record-field-input";
import { useRecordMutations } from "@/hooks/use-records";
import { describeFailure } from "@/lib/api";

interface RecordDrawerProps {
  collection: CollectionModel;
  record: RecordModel | null;
  open: boolean;
  onOpenChange: (open: boolean) => void;
}

type FileFieldState = Record<string, File[]>;

export function RecordDrawer({ collection, record, open, onOpenChange }: RecordDrawerProps) {
  const isNew = record === null;
  const identityField = (collection.type === "auth" ? collection.passwordAuth?.identityFields?.[0] : undefined) ?? "email";
  const [values, setValues] = useState<Record<string, unknown>>({});
  const [files, setFiles] = useState<FileFieldState>({});
  const [errors, setErrors] = useState<Record<string, string>>({});
  const { create, update } = useRecordMutations(collection.name);
  const fields = userFields(collection);

  useEffect(() => {
    if (!open) return;
    const initial: Record<string, unknown> = {};
    for (const field of fields) initial[field.name] = existingRecordValue(record, field);
    if (collection.type === "auth") {
      initial[identityField] = (record?.[identityField] as string | undefined) ?? "";
    }
    setValues(initial);
    setFiles({});
    setErrors({});
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open, record, collection, identityField]);

  function buildPayload(): Record<string, unknown> | FormData {
    const hasFile = fields.some((f) => f.type === "file");
    if (!hasFile) return values;

    const form = new FormData();
    for (const [key, value] of Object.entries(values)) {
      if (value === null || value === undefined) continue;
      if (Array.isArray(value)) {
        for (const item of value) form.append(key, typeof item === "string" ? item : JSON.stringify(item));
      } else {
        form.append(key, typeof value === "string" ? value : JSON.stringify(value));
      }
    }
    for (const [fieldName, fileList] of Object.entries(files)) {
      for (const file of fileList) form.append(fieldName, file);
    }
    return form;
  }

  const pending = create.isPending || update.isPending;

  async function handleSubmit(event: React.FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (pending) return;
    setErrors({});
    try {
      if (isNew) {
        await create.mutateAsync(buildPayload());
        toast.success("Record created");
      } else {
        await update.mutateAsync({ id: record.id, data: buildPayload() });
        toast.success("Record updated");
      }
      onOpenChange(false);
    } catch (error) {
      const failure = describeFailure(error);
      if (Object.keys(failure.fields).length > 0) {
        setErrors(failure.fields);
        toast.error("Fix the highlighted fields");
      } else {
        toast.error(failure.title, { description: failure.detail || undefined });
      }
    }
  }

  return (
    <Sheet open={open} onOpenChange={onOpenChange}>
      <SheetContent side="right" className="gap-0 p-0 data-[side=right]:w-full data-[side=right]:sm:max-w-[440px]">
        <form onSubmit={handleSubmit} className="flex h-full min-h-0 flex-col">
          <SheetHeader>
            <SheetTitle>{isNew ? `New ${collection.name.replace(/s$/, "")}` : "Edit record"}</SheetTitle>
            <SheetDescription>
              {isNew ? `Add a row to ${collection.name}.` : `Editing a row in ${collection.name}.`}
            </SheetDescription>
          </SheetHeader>

          <div className="flex min-h-0 flex-1 flex-col gap-4 overflow-y-auto px-4 pb-4">
            {collection.type === "auth" ? (
              <div className="flex flex-col gap-1.5">
                <Label htmlFor="record-identity" className="capitalize">
                  {identityField}
                </Label>
                <Input
                  id="record-identity"
                  type={identityField === "email" ? "email" : "text"}
                  value={(values[identityField] as string) ?? ""}
                  onChange={(e) => setValues((v) => ({ ...v, [identityField]: e.target.value }))}
                  aria-invalid={errors[identityField] ? true : undefined}
                  className="h-control-md"
                />
                {errors[identityField] ? <p className="text-xs text-destructive">{errors[identityField]}</p> : null}
              </div>
            ) : null}

            {collection.type === "auth" ? (
              <div className="flex flex-col gap-1.5">
                <Label htmlFor="record-password">{isNew ? "Password" : "New password"}</Label>
                <Input
                  id="record-password"
                  type="password"
                  value={(values.password as string) ?? ""}
                  onChange={(e) => setValues((v) => ({ ...v, password: e.target.value }))}
                  placeholder={isNew ? undefined : "Leave blank to keep current"}
                  aria-invalid={errors.password ? true : undefined}
                  className="h-control-md"
                />
                {errors.password ? <p className="text-xs text-destructive">{errors.password}</p> : null}
              </div>
            ) : null}

            {fields.map((field) => (
              <div key={field.id} className="flex flex-col gap-1.5">
                <Label>
                  {field.name}
                  {field.required ? <span className="text-primary"> *</span> : null}
                </Label>
                {field.type === "file" ? (
                  <div className="flex flex-col gap-1.5">
                    <input
                      type="file"
                      aria-label={field.name}
                      multiple={isMultiValue(field)}
                      onChange={(e) => setFiles((f) => ({ ...f, [field.name]: Array.from(e.target.files ?? []) }))}
                      className="text-sm text-muted-foreground file:mr-3 file:rounded-md file:border-0 file:bg-secondary file:px-2.5 file:py-1.5 file:text-sm file:font-medium file:text-secondary-foreground"
                    />
                    {!isNew && record && record[field.name] ? (
                      <p className="text-xs text-muted-foreground">
                        Current: {Array.isArray(record[field.name]) ? (record[field.name] as string[]).join(", ") : String(record[field.name])}.
                        Choosing a new file replaces it.
                      </p>
                    ) : null}
                  </div>
                ) : (
                  <RecordFieldInput
                    field={field}
                    value={values[field.name]}
                    onChange={(value) => setValues((v) => ({ ...v, [field.name]: value }))}
                    error={errors[field.name]}
                  />
                )}
                {errors[field.name] ? <p className="text-xs text-destructive">{errors[field.name]}</p> : null}
              </div>
            ))}

            {pending ? <Progress value={null} label="Saving" /> : null}
          </div>

          <SheetFooter className="flex-row justify-end border-t border-border">
            <Button type="submit" disabled={pending}>
              {pending ? <Spinner /> : null}
              {pending ? "Saving…" : isNew ? "Create" : "Save changes"}
            </Button>
          </SheetFooter>
        </form>
      </SheetContent>
    </Sheet>
  );
}
