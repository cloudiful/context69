import { Input } from "@/components/ui/input";

import { FormField } from "./form-field";

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
}) {
  return (
    <FormField label={label} htmlFor={id} helper={helper}>
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
