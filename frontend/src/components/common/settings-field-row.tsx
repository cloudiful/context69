import type { ReactNode } from "react";

import { FormField } from "./form-field";

/**
 * One settings row: a label column on the left and the value control on the
 * right on desktop, collapsing to a stacked row on narrow screens. Use this
 * for controls that are not one of the shared text/number/toggle fields.
 */
export function SettingsFieldRow({
  label,
  htmlFor,
  helper,
  error,
  controlClassName,
  className,
  children,
}: {
  label: string;
  htmlFor?: string;
  helper?: string;
  error?: string | null;
  controlClassName?: string;
  className?: string;
  children: ReactNode;
}) {
  return (
    <FormField
      layout="row"
      label={label}
      htmlFor={htmlFor}
      helper={helper}
      error={error}
      controlClassName={controlClassName}
      className={className}
    >
      {children}
    </FormField>
  );
}
