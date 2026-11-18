import { Input } from "@/components/ui/input";

import { FormField, type FieldLayout } from "./form-field";

export function TextField({
  id,
  label,
  value,
  onChange,
  type = "text",
  placeholder,
  helper,
  autoComplete,
  readOnly,
  disabled,
  testId,
  className,
  layout,
  controlClassName,
}: {
  id: string;
  label: string;
  value: string;
  onChange: (value: string) => void;
  type?: string;
  placeholder?: string;
  helper?: string;
  autoComplete?: string;
  readOnly?: boolean;
  disabled?: boolean;
  testId?: string;
  className?: string;
  layout?: FieldLayout;
  controlClassName?: string;
}) {
  return (
    <FormField
      label={label}
      htmlFor={id}
      helper={helper}
      className={className}
      layout={layout}
      controlClassName={controlClassName}
    >
      <Input
        id={id}
        name={id}
        type={type}
        value={value}
        placeholder={placeholder}
        autoComplete={autoComplete}
        readOnly={readOnly}
        disabled={disabled}
        data-testid={testId}
        onChange={(event) => onChange(event.target.value)}
      />
    </FormField>
  );
}
