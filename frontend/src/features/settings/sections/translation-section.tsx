import { ArrowDown, ArrowUp, Pencil } from "lucide-react";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

import { NumberField } from "@/components/common/number-field";
import { FormField } from "@/components/common/form-field";
import { SettingsBlock, SettingsSection } from "@/components/common/settings-section";
import { TablePagination } from "@/components/common/table-pagination";
import { TextField } from "@/components/common/text-field";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Switch } from "@/components/ui/switch";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";
import { SettingsGate } from "@/features/settings/settings-gate";
import { useSettings } from "@/features/settings/settings-context";
import { useLlmProviderProbe } from "@/features/settings/use-llm-provider-probe";
import { apiClient } from "@/services/api";
import type { Pagination, TranslationProviderPageResponse } from "@/services/api";

const PAGE_SIZE = 50;
const EMPTY_PAGINATION: Pagination = { page: 1, page_size: PAGE_SIZE, total: 0, total_pages: 0 };

const PROVIDER_LABELS: Record<string, string> = {
  deepl: "DeepL",
  llm: "LLM",
  libretranslate: "LibreTranslate",
};

const LLM_KINDS = [
  { value: "openai_responses", label: "OpenAI Responses" },
  { value: "openai_chat_completions", label: "OpenAI Chat Completions" },
  { value: "anthropic_messages", label: "Anthropic Messages" },
];

const DEEPL_PLANS = [
  { value: "free", label: "Developer" },
  { value: "pro", label: "Growth / Enterprise" },
];

type ProviderDraft = ReturnType<typeof useSettings>["page"]["translationProviders"][number];

export function TranslationSection() {
  const { t } = useTranslation();
  const { page } = useSettings();
  const providers = page.translationProviders;

  const [providerPage, setProviderPage] = useState<TranslationProviderPageResponse | null>(null);
  const [pageNumber, setPageNumber] = useState(1);
  const [pageSize, setPageSize] = useState(PAGE_SIZE);
  const [loading, setLoading] = useState(false);
  const [loadFailed, setLoadFailed] = useState(false);
  const [editing, setEditing] = useState<ProviderDraft | null>(null);
  const [editingIndex, setEditingIndex] = useState(-1);

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    setLoadFailed(false);
    apiClient.listTranslationProviders({ page: pageNumber, pageSize })
      .then((response) => {
        if (!cancelled) {
          setProviderPage(response);
        }
      })
      .catch(() => {
        if (!cancelled) {
          setProviderPage(null);
          setLoadFailed(true);
        }
      })
      .finally(() => {
        if (!cancelled) {
          setLoading(false);
        }
      });
    return () => {
      cancelled = true;
    };
  }, [pageNumber, pageSize]);

  const pagedProviders = (providerPage?.items ?? [])
    .map((item) => providers.find((provider) => provider.provider === item.provider))
    .filter((provider): provider is ProviderDraft => !!provider);

  function providerIndex(provider: ProviderDraft) {
    return providers.findIndex((item) => item.provider === provider.provider);
  }

  function move(provider: ProviderDraft, direction: -1 | 1) {
    const index = providerIndex(provider);
    const target = index + direction;
    if (index < 0 || target < 0 || target >= providers.length) {
      return;
    }
    page.updateTranslationProviders((current) => {
      const [movedProvider] = current.splice(index, 1);
      current.splice(target, 0, movedProvider);
      return current.map((item, priority) => ({ ...item, priority }));
    });
  }

  function quotaLabel(provider: ProviderDraft) {
    if (provider.provider === "libretranslate") {
      return t("settings.translation.unlimited");
    }
    return provider.monthly_character_limit?.toLocaleString() ?? t("settings.translation.unlimited");
  }

  function usageLabel(provider: ProviderDraft) {
    return provider.provider === "deepl"
      ? t("settings.translation.lifetimeUsage", { count: provider.current_month_characters })
      : t("settings.translation.currentUsage", { count: provider.current_month_characters });
  }

  function changePage(next: number) {
    setPageNumber(next);
  }

  function changePageSize(next: number) {
    if (next === pageSize) {
      return;
    }
    setPageSize(next);
    setPageNumber(1);
  }

  return (
    <SettingsSection
      title={t("settings.translation.title")}
      description={t("settings.translation.description")}
    >
      <SettingsGate>
      <SettingsBlock compact>
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead>{t("settings.translation.provider")}</TableHead>
              <TableHead>{t("settings.translation.enabled")}</TableHead>
              <TableHead>{t("settings.translation.quota")}</TableHead>
              <TableHead>{t("settings.translation.usage")}</TableHead>
              <TableHead>{t("settings.translation.configuration")}</TableHead>
              <TableHead>{t("common.actions")}</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {pagedProviders.length === 0 ? (
              <TableRow>
                <TableCell colSpan={6} className="text-sm text-muted-foreground">
                  {loading ? t("common.loading") : loadFailed ? t("settings.translation.loadFailed") : ""}
                </TableCell>
              </TableRow>
            ) : (
              pagedProviders.map((provider) => {
                const index = providerIndex(provider);
                return (
                  <TableRow key={provider.provider}>
                    <TableCell className="font-medium">{PROVIDER_LABELS[provider.provider] ?? provider.provider}</TableCell>
                    <TableCell>
                      <Switch
                        id={`translation-${provider.provider}-enabled`}
                        aria-label={PROVIDER_LABELS[provider.provider] ?? provider.provider}
                        checked={provider.enabled}
                        onCheckedChange={(checked) => page.updateTranslationProviders((current) => current.map((item) => (
                          item.provider === provider.provider ? { ...item, enabled: checked } : item
                        )))}
                      />
                    </TableCell>
                    <TableCell className="whitespace-nowrap">{quotaLabel(provider)}</TableCell>
                    <TableCell className="whitespace-nowrap text-muted-foreground">{usageLabel(provider)}</TableCell>
                    <TableCell>
                      <Badge variant={provider.endpoint ? "outline" : "secondary"} className="whitespace-nowrap">
                        {provider.endpoint ? t("settings.translation.configured") : t("settings.translation.notConfigured")}
                      </Badge>
                    </TableCell>
                    <TableCell>
                      <div className="flex items-center gap-1">
                        <Button
                          type="button"
                          variant="ghost"
                          size="icon-sm"
                          aria-label={t("common.edit")}
                          onClick={() => {
                            setEditing({ ...provider });
                            setEditingIndex(index);
                          }}
                        >
                          <Pencil aria-hidden="true" />
                        </Button>
                        <Button
                          type="button"
                          variant="ghost"
                          size="icon-sm"
                          aria-label={t("common.move")}
                          disabled={index <= 0}
                          onClick={() => move(provider, -1)}
                        >
                          <ArrowUp aria-hidden="true" />
                        </Button>
                        <Button
                          type="button"
                          variant="ghost"
                          size="icon-sm"
                          aria-label={t("common.move")}
                          disabled={index >= providers.length - 1}
                          onClick={() => move(provider, 1)}
                        >
                          <ArrowDown aria-hidden="true" />
                        </Button>
                      </div>
                    </TableCell>
                  </TableRow>
                );
              })
            )}
          </TableBody>
        </Table>
        <TablePagination
          pagination={providerPage?.pagination ?? EMPTY_PAGINATION}
          onPage={changePage}
          onPageSize={changePageSize}
        />
      </SettingsBlock>
      </SettingsGate>

      <ProviderDialog
        editing={editing}
        onClose={() => setEditing(null)}
        onApply={(provider) => {
          page.updateTranslationProviders((current) => {
            const next = [...current];
            if (editingIndex >= 0 && editingIndex < next.length) {
              next[editingIndex] = provider;
            }
            return next;
          });
          setEditing(null);
        }}
      />
    </SettingsSection>
  );
}

function ProviderDialog({
  editing,
  onClose,
  onApply,
}: {
  editing: ProviderDraft | null;
  onClose: () => void;
  onApply: (provider: ProviderDraft) => void;
}) {
  const { t } = useTranslation();
  const probe = useLlmProviderProbe();
  const [draft, setDraft] = useState<ProviderDraft | null>(editing);

  useEffect(() => {
    setDraft(editing ? { ...editing } : null);
    probe.reset();
  }, [editing]);

  function update(patch: Partial<ProviderDraft>) {
    setDraft((current) => (current ? { ...current, ...patch } : current));
    probe.reset();
  }

  const probeUsesStoredKey = !draft?.api_key?.trim();

  const outcomeKey = probe.result
    ? {
      success: "settings.translation.llmProbeResultSuccess",
      auth_error: "settings.translation.llmProbeResultAuthError",
      not_found: "settings.translation.llmProbeResultNotFound",
      route_error: "settings.translation.llmProbeResultRouteError",
      rate_limited: "settings.translation.llmProbeResultRateLimited",
      invalid_payload: "settings.translation.llmProbeResultInvalidPayload",
      upstream_error: "settings.translation.llmProbeResultUpstreamError",
      timeout: "settings.translation.llmProbeResultTimeout",
      transport_error: "settings.translation.llmProbeResultTransportError",
      configuration_error: "settings.translation.llmProbeResultConfigurationError",
    }[probe.result.outcome] ?? null
    : null;

  const probeVariant = probe.result?.outcome === "success"
    ? "default"
    : probe.result && ["timeout", "rate_limited"].includes(probe.result.outcome)
      ? "secondary"
      : "destructive";

  return (
    <Dialog open={editing !== null} onOpenChange={(open) => {
      if (!open) {
        onClose();
      }
    }}>
      <DialogContent className="w-[38rem] max-w-[96vw]">
        <DialogHeader>
          <DialogTitle>{draft ? PROVIDER_LABELS[draft.provider] ?? draft.provider : ""}</DialogTitle>
          {draft?.provider === "llm" ? (
            <DialogDescription>{t("settings.translation.llmDescription")}</DialogDescription>
          ) : null}
        </DialogHeader>

        {draft ? (
          <div className="grid gap-3">
            <TextField
              id={`translation-${draft.provider}-endpoint`}
              type="url"
              label={t("settings.translation.endpoint")}
              value={draft.endpoint ?? ""}
              onChange={(value) => update({ endpoint: value })}
            />
            <TextField
              id={`translation-${draft.provider}-api-key`}
              type="password"
              autoComplete="new-password"
              label={t("settings.translation.apiKey")}
              placeholder={draft.has_api_key ? t("settings.translation.keyStored") : ""}
              value={draft.api_key ?? ""}
              onChange={(value) => update({ api_key: value })}
            />
            {draft.provider === "llm" ? (
              <>
                <TextField
                  id="translation-llm-model"
                  label={t("settings.translation.model")}
                  value={draft.model ?? ""}
                  onChange={(value) => update({ model: value })}
                />
                <FormField label={t("settings.translation.apiKind")} htmlFor="translation-llm-api-kind">
                  <Select value={draft.llm_api_kind ?? ""} onValueChange={(value) => update({ llm_api_kind: value as ProviderDraft["llm_api_kind"] })}>
                    <SelectTrigger id="translation-llm-api-kind">
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      {LLM_KINDS.map((kind) => (
                        <SelectItem key={kind.value} value={kind.value}>{kind.label}</SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                </FormField>
              </>
            ) : null}
            {draft.provider === "deepl" ? (
              <FormField label={t("settings.translation.plan")} htmlFor="translation-deepl-plan">
                <Select value={draft.deepl_plan ?? ""} onValueChange={(value) => update({ deepl_plan: value as ProviderDraft["deepl_plan"] })}>
                  <SelectTrigger id="translation-deepl-plan">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    {DEEPL_PLANS.map((plan) => (
                      <SelectItem key={plan.value} value={plan.value}>{plan.label}</SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </FormField>
            ) : null}
            {draft.provider !== "libretranslate" ? (
              <NumberField
                id={`translation-${draft.provider}-quota`}
                label={draft.provider === "deepl"
                  ? t("settings.translation.lifetimeQuota")
                  : t("settings.translation.monthlyQuota")}
                min={1}
                step={1000}
                value={draft.monthly_character_limit ?? 0}
                onChange={(value) => update({ monthly_character_limit: value })}
              />
            ) : null}

            {draft.provider === "llm" ? (
              <div className="grid gap-2 border-t pt-3">
                <p className="text-xs text-muted-foreground">{t("settings.translation.llmProbeWarning")}</p>
                <p className="text-xs text-muted-foreground">{t("settings.translation.llmProbeEndpointTip")}</p>
                {probeUsesStoredKey ? (
                  <p className="text-xs text-muted-foreground">{t("settings.translation.llmProbeStoredKey")}</p>
                ) : null}
                <div>
                  <Button
                    type="button"
                    variant="outline"
                    disabled={probe.pending}
                    aria-busy={probe.pending}
                    onClick={() => void probe.submit({
                      endpoint: draft.endpoint ?? "",
                      model: draft.model ?? "",
                      llm_api_kind: draft.llm_api_kind ?? "",
                      api_key: draft.api_key?.trim() ? draft.api_key : undefined,
                    })}
                  >
                    {t("settings.translation.testLlmProvider")}
                  </Button>
                </div>
                {probe.result && outcomeKey ? (
                  <Alert variant={probeVariant === "destructive" ? "destructive" : "default"}>
                    <AlertTitle>{t(outcomeKey)}</AlertTitle>
                    <AlertDescription className="whitespace-pre-line">
                      {[
                        probe.result.safe_endpoint
                          ? t("settings.translation.llmProbeEndpoint", { endpoint: probe.result.safe_endpoint })
                          : null,
                        probe.result.http_status != null
                          ? t("settings.translation.llmProbeStatus", { status: probe.result.http_status })
                          : null,
                        t("settings.translation.llmProbeLatency", { ms: probe.result.latency_ms }),
                        probe.result.outcome === "route_error"
                          ? t("settings.translation.llmProbeRouteHint")
                          : null,
                      ].filter((line): line is string => line !== null).join("\n")}
                    </AlertDescription>
                  </Alert>
                ) : null}
              </div>
            ) : null}
          </div>
        ) : null}

        <DialogFooter>
          <Button type="button" variant="outline" onClick={onClose}>{t("common.cancel")}</Button>
          <Button
            type="button"
            onClick={() => {
              if (draft) {
                onApply(draft);
              }
            }}
          >
            {t("common.confirm")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
