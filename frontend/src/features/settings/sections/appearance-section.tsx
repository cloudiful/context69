import { Check, ChevronsUpDown } from "lucide-react";
import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";

import { FormField } from "@/components/common/form-field";
import { SettingsBlock, SettingsSection } from "@/components/common/settings-section";
import { Button } from "@/components/ui/button";
import {
  Command,
  CommandEmpty,
  CommandGroup,
  CommandInput,
  CommandItem,
  CommandList,
} from "@/components/ui/command";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { SettingsGate } from "@/features/settings/settings-gate";
import { useSettings } from "@/features/settings/settings-context";
import type { ThemePreference } from "@/lib/preferences-store";
import { AUTOMATIC_TIME_ZONE, browserTimeZone, isValidTimeZone, listSupportedTimeZones } from "@/lib/time-zone";
import { cn } from "@/lib/utils";

export function AppearanceSection() {
  const { t } = useTranslation();
  const { page } = useSettings();
  const { draft, updateLocale, updateTheme, updateTimeZone } = page.appearance;

  return (
    <SettingsSection title={t("settings.appearance.title")}>
      <SettingsGate>
      <SettingsBlock id="settings-appearance">
        <div className="grid max-w-sm gap-3">
          <FormField label={t("language.label")} htmlFor="settings-locale-select">
            <Select value={draft.locale} onValueChange={(value) => updateLocale(value)}>
              <SelectTrigger id="settings-locale-select" data-testid="settings-locale-select" aria-label={t("language.label")}>
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="en">{t("language.en")}</SelectItem>
                <SelectItem value="zh-CN">{t("language.zhCN")}</SelectItem>
              </SelectContent>
            </Select>
          </FormField>

          <FormField label={t("theme.label")} htmlFor="settings-theme-select">
            <Select value={draft.theme} onValueChange={(value) => updateTheme(value as ThemePreference)}>
              <SelectTrigger id="settings-theme-select" data-testid="settings-theme-select" aria-label={t("theme.label")}>
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="system">{t("settings.appearance.themeSystem")}</SelectItem>
                <SelectItem value="light">{t("theme.light")}</SelectItem>
                <SelectItem value="dark">{t("theme.dark")}</SelectItem>
              </SelectContent>
            </Select>
          </FormField>

          <TimeZoneCombobox
            value={draft.timeZone}
            onChange={updateTimeZone}
          />
        </div>
      </SettingsBlock>
      </SettingsGate>
    </SettingsSection>
  );
}

function TimeZoneCombobox({
  value,
  onChange,
}: {
  value: string;
  onChange: (value: string) => void;
}) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState("");

  const options = useMemo(() => {
    const automatic = t("settings.appearance.timeZoneAutomatic", { zone: browserTimeZone });
    const selected = value === AUTOMATIC_TIME_ZONE || isValidTimeZone(value) ? value : AUTOMATIC_TIME_ZONE;
    const zones = listSupportedTimeZones(selected).map((zone) => ({ value: zone, label: zone }));
    return [{ value: AUTOMATIC_TIME_ZONE, label: automatic }, ...zones];
  }, [t, value]);

  const filtered = useMemo(() => {
    const needle = query.trim().toLowerCase();
    if (!needle) {
      return options;
    }
    return options.filter((option) => option.label.toLowerCase().includes(needle));
  }, [options, query]);

  const active = value === AUTOMATIC_TIME_ZONE || isValidTimeZone(value) ? value : AUTOMATIC_TIME_ZONE;
  const label = options.find((option) => option.value === active)?.label ?? active;

  return (
    <FormField label={t("settings.appearance.timeZoneLabel")} htmlFor="settings-time-zone-select">
      <Popover open={open} onOpenChange={setOpen}>
        <PopoverTrigger asChild>
          <Button
            id="settings-time-zone-select"
            type="button"
            variant="outline"
            role="combobox"
            aria-expanded={open}
            aria-controls="settings-time-zone-list"
            data-testid="settings-time-zone-select"
            className="justify-between font-normal"
          >
            <span className="truncate">{label}</span>
            <ChevronsUpDown aria-hidden="true" className="opacity-50" />
          </Button>
        </PopoverTrigger>
        <PopoverContent className="w-[--radix-popover-trigger-width] p-0" align="start">
          <Command shouldFilter={false}>
            <CommandInput
              value={query}
              onValueChange={setQuery}
              placeholder={t("settings.appearance.timeZoneSearchPlaceholder")}
            />
            <CommandList id="settings-time-zone-list">
              <CommandEmpty>{t("settings.appearance.timeZoneEmpty")}</CommandEmpty>
              <CommandGroup>
                {filtered.map((option) => (
                  <CommandItem
                    key={option.value}
                    value={option.value}
                    data-testid={`settings-time-zone-option-${option.value}`}
                    onSelect={() => {
                      onChange(option.value);
                      setQuery("");
                      setOpen(false);
                    }}
                  >
                    <span className="truncate">{option.label}</span>
                    <Check
                      aria-hidden="true"
                      className={cn("ml-auto", option.value === active ? "opacity-100" : "opacity-0")}
                    />
                  </CommandItem>
                ))}
              </CommandGroup>
            </CommandList>
          </Command>
        </PopoverContent>
      </Popover>
    </FormField>
  );
}
