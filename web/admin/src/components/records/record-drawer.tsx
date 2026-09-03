import { useEffect, useState } from "react";
import { toast } from "sonner";
import type { CollectionModel, RecordModel } from "cratebase";
import { ClientResponseError } from "cratebase";
import { Drawer } from "@/components/interior/drawer";
import { LoadingButton } from "@/components/interior/loading-button";
import { ProgressBar } from "@/components/interior/progress-bar";
import { RecordFieldInput, existingRecordValue } from "@/components/records/record-field-input";
import { useRecordMutations } from "@/hooks/use-records";

interface RecordDrawerProps {
  collection: CollectionModel;
  record: RecordModel | null;
  open: boolean;
  onOpenChange: (open: boolean) => void;
}

type FileFieldState = Record<string, File[]>;

export function RecordDrawer({ collection, record, open, onOpenChange }: RecordDrawerProps) {
  const isNew = record === null;
  const identityField = (collection.authOptions?.identityField as string | undefined) ?? "email";
  const [values, setValues] = useState<Record<string, unknown>>({});
  const [files, setFiles] = useState<FileFieldState>({});
  const [errors, setErrors] = useState<Record<string, string>>({});
  const { create, update } = useRecordMutations(collection.name);

  useEffect(() => {
    if (!open) return;
    const initial: Record<string, unknown> = {};
    for (const field of collection.schema) initial[field.name] = existingRecordValue(record, field);
    if (collection.type === "auth") {
      initial[identityField] = (record?.[identityField] as string | undefined) ?? "";
    }
    setValues(initial);
    setFiles({});
    setErrors({});
  }, [open, record, collection]);

  function buildPayload(): Record<string, unknown> | FormData {
    const hasFile = collection.schema.some((f) => f.type === "file");
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

  async function handleSubmit() {
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
      if (error instanceof ClientResponseError && error.status === 400) {
        setErrors(error.data);
        toast.error("Fix the highlighted fields");
      } else {
        toast.error(error instanceof Error ? error.message : "Something went wrong");
      }
      throw error;
    }
  }

  const pending = create.isPending || update.isPending;

  return (
    <Drawer
      open={open}
      onOpenChange={onOpenChange}
      title={isNew ? `New ${collection.name.replace(/s$/, "")}` : "Edit record"}
      width={440}
    >
      <div className="flex flex-col gap-4">
        {collection.type === "auth" ? (
          <div className="flex flex-col gap-1.5">
            <label className="text-[13px] font-medium text-foreground capitalize">{identityField}</label>
            <input
              type={identityField === "email" ? "email" : "text"}
              value={(values[identityField] as string) ?? ""}
              onChange={(e) => setValues((v) => ({ ...v, [identityField]: e.target.value }))}
              className={`h-9 w-full rounded-[9px] border-2 bg-secondary/60 px-2.5 text-[13px] outline-none focus:bg-card ${
                errors[identityField] ? "border-destructive" : "border-border focus:border-primary"
              }`}
            />
            {errors[identityField] ? <p className="text-[11.5px] text-destructive">{errors[identityField]}</p> : null}
          </div>
        ) : null}

        {collection.type === "auth" ? (
          <div className="flex flex-col gap-1.5">
            <label className="text-[13px] font-medium text-foreground">{isNew ? "Password" : "New password"}</label>
            <input
              type="password"
              value={(values.password as string) ?? ""}
              onChange={(e) => setValues((v) => ({ ...v, password: e.target.value }))}
              placeholder={isNew ? undefined : "Leave blank to keep current"}
              className={`h-9 w-full rounded-[9px] border-2 bg-secondary/60 px-2.5 text-[13px] outline-none focus:bg-card ${
                errors.password ? "border-destructive" : "border-border focus:border-primary"
              }`}
            />
            {errors.password ? <p className="text-[11.5px] text-destructive">{errors.password}</p> : null}
          </div>
        ) : null}

        {collection.schema.map((field) => (
          <div key={field.id} className="flex flex-col gap-1.5">
            <label className="text-[13px] font-medium text-foreground">
              {field.name}
              {field.required ? <span className="text-primary"> *</span> : null}
            </label>
            {field.type === "file" ? (
              <div className="flex flex-col gap-1.5">
                <input
                  type="file"
                  multiple={Boolean(field.options?.multiple)}
                  onChange={(e) => setFiles((f) => ({ ...f, [field.name]: Array.from(e.target.files ?? []) }))}
                  className="text-[12.5px] text-muted-foreground file:mr-3 file:rounded-md file:border-0 file:bg-secondary file:px-2.5 file:py-1.5 file:text-[12px] file:font-medium file:text-secondary-foreground"
                />
                {!isNew && record && record[field.name] ? (
                  <p className="text-[11.5px] text-muted-foreground">
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
            {errors[field.name] ? <p className="text-[11.5px] text-destructive">{errors[field.name]}</p> : null}
          </div>
        ))}

        {pending ? <ProgressBar value={null} label="Saving" /> : null}
      </div>

      <div className="mt-6 flex justify-end">
        <LoadingButton
          onAction={handleSubmit}
          pendingLabel="Saving…"
          successLabel="Saved"
          errorLabel="Fix errors"
          className="!border-primary !bg-primary !px-4 !text-primary-foreground hover:!bg-primary/90 dark:!border-primary dark:!bg-primary dark:!text-primary-foreground dark:hover:!bg-primary/90"
        >
          {isNew ? "Create" : "Save changes"}
        </LoadingButton>
      </div>
    </Drawer>
  );
}
