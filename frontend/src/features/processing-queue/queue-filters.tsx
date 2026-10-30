import { Check, ListFilter } from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSub,
  DropdownMenuSubContent,
  DropdownMenuSubTrigger,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { cn } from "@/lib/utils";

export interface QueueFilterOption {
  label: string;
  value: string | null;
  children?: QueueFilterOption[];
}

/**
 * One column's filter as a separate icon action, so the column title stays
 * visible once. A filtered header stays visually marked, which is the only sign
 * that the visible page is a narrowed slice.
 */
function selectedLabel(options: QueueFilterOption[], selected: string | null): string {
  for (const option of options) {
    if (option.value === selected) {
      return option.label;
    }
    const child = option.children?.find((entry) => entry.value === selected);
    if (child) {
      return child.label;
    }
  }
  return "";
}

export function QueueHeaderFilter({
  label,
  options,
  selected,
  active,
  disabled,
  onSelect,
}: {
  label: string;
  options: QueueFilterOption[];
  selected: string | null;
  active: boolean;
  disabled?: boolean;
  onSelect: (value: string | null, group?: string | null) => void;
}) {
  const selectedText = selectedLabel(options, selected);
  const ariaLabel = active && selectedText ? `${label}: ${selectedText}` : label;

  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button
          type="button"
          variant="ghost"
          size="icon-sm"
          className={cn("size-6 shrink-0", active ? "text-foreground" : "text-muted-foreground")}
          aria-label={ariaLabel}
          data-filter-active={active ? "true" : "false"}
          disabled={disabled}
        >
          <ListFilter aria-hidden="true" className={cn("size-3", active ? "opacity-100" : "opacity-60")} />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="start" className="min-w-40">
        {options.map((option) => (
          option.children
            ? (
                <DropdownMenuSub key={option.value ?? option.label}>
                  <DropdownMenuSubTrigger>{option.label}</DropdownMenuSubTrigger>
                  <DropdownMenuSubContent>
                    {option.children.map((child) => (
                      <DropdownMenuItem
                        key={child.value ?? child.label}
                        onSelect={() => onSelect(child.value, option.value)}
                      >
                        <OptionMark selected={selected === child.value} />
                        {child.label}
                      </DropdownMenuItem>
                    ))}
                  </DropdownMenuSubContent>
                </DropdownMenuSub>
              )
            : (
                <DropdownMenuItem key={option.value ?? option.label} onSelect={() => onSelect(option.value)}>
                  <OptionMark selected={selected === option.value} />
                  {option.label}
                </DropdownMenuItem>
              )
        ))}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

function OptionMark({ selected }: { selected: boolean }) {
  return (
    <Check
      aria-hidden="true"
      className={cn("size-4", selected ? "opacity-100" : "opacity-0")}
    />
  );
}
