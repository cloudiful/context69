import { Input } from "@/components/ui/input";

import { FormField, type FieldLayout } from "./form-field";

export function NumberField({
  id,
  label,
  value,
  onChange,
  min,
  max,
  step = 1,
  helper,
  disabled,
  testId,
  layout,
  controlClassName = "max-w-28",
}: {
  id: string;
  label: string;
  value: number;
  onChange: (value: number) => void;
  min?: number;
  max?: number;
  step?: number;
  helper?: string;
  disabled?: boolean;
  testId?: string;
  layout?: FieldLayout;
  controlClassName?: string;
}) {
  return (
    <FormField
      label={label}
      htmlFor={id}
      helper={helper}
      layout={layout}
      controlClassName={controlClassName}
    >
      <Input
        id={id}
        name={id}
        type="number"
        inputMode="decimal"
        value={Number.isFinite(value) ? String(value) : ""}
        min={min}
        max={max}
        step={step}
        disabled={disabled}
        data-testid={testId}
        onChange={(event) => {
          const parsed = Number(event.target.value);
          onChange(Number.isFinite(parsed) ? parsed : 0);
        }}
      />
    </FormField>
  );
}
