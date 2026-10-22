import { Switch } from "@/components/ui/switch";
import { cn } from "@/lib/utils";

export function ToggleField({
  id,
  label,
  checked,
  onCheckedChange,
  helper,
  testId,
  disabled,
  className,
}: {
  id: string;
  label: string;
  checked: boolean;
  onCheckedChange: (checked: boolean) => void;
  helper?: string;
  testId?: string;
  disabled?: boolean;
  className?: string;
}) {
  return (
    <div className={cn("flex items-start gap-2", className)}>
      <Switch
        id={id}
        data-testid={testId}
        checked={checked}
        disabled={disabled}
        onCheckedChange={onCheckedChange}
      />
      <div className="grid gap-0.5">
        <label htmlFor={id} className="text-sm leading-tight">
          {label}
        </label>
        {helper ? <p className="text-xs text-muted-foreground">{helper}</p> : null}
      </div>
    </div>
  );
}
