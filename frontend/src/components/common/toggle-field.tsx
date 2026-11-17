import { Switch } from "@/components/ui/switch";
import { cn } from "@/lib/utils";

import {
  FIELD_ROW_GRID_CLASS,
  FIELD_ROW_LABEL_CLASS,
  type FieldLayout,
} from "./form-field";

export function ToggleField({
  id,
  label,
  checked,
  onCheckedChange,
  helper,
  testId,
  disabled,
  className,
  layout = "stacked",
}: {
  id: string;
  label: string;
  checked: boolean;
  onCheckedChange: (checked: boolean) => void;
  helper?: string;
  testId?: string;
  disabled?: boolean;
  className?: string;
  layout?: FieldLayout;
}) {
  const control = (
    <Switch
      id={id}
      data-testid={testId}
      checked={checked}
      disabled={disabled}
      onCheckedChange={onCheckedChange}
    />
  );
  const labelText = (
    <label htmlFor={id} className="text-sm leading-tight">
      {label}
    </label>
  );
  const helperText = helper ? <p className="text-xs text-muted-foreground">{helper}</p> : null;

  if (layout === "row") {
    return (
      <div className={cn(FIELD_ROW_GRID_CLASS, className)}>
        <label htmlFor={id} className={FIELD_ROW_LABEL_CLASS}>
          {label}
        </label>
        <div className="grid min-w-0 gap-1">
          <div className="w-full max-w-md">{control}</div>
          {helperText}
        </div>
      </div>
    );
  }

  return (
    <div className={cn("flex items-start gap-2", className)}>
      {control}
      <div className="grid gap-0.5">
        {labelText}
        {helperText}
      </div>
    </div>
  );
}
