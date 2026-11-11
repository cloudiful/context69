import { useForm } from "@tanstack/react-form";
import { useQuery } from "@tanstack/react-query";
import { useState } from "react";
import { useTranslation } from "react-i18next";

import { FormField } from "@/components/common/form-field";
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
import { Input } from "@/components/ui/input";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { ApiError, apiClient, type GitProviderKind, type GitRepositorySource, type GitWebhookOwnership } from "@/services/api";
import { resolveApiUrl } from "@/lib/api-base";

import { required } from "./fields";

const WEBHOOK_OWNERSHIPS: GitWebhookOwnership[] = ["integration", "external", "unknown"];

type WebhookFormValues = {
  externalHookId: string;
  ownership: GitWebhookOwnership;
  signingSecret: string;
};

export function WebhookDialog({
  open,
  groupPath,
  source,
  busy,
  onOpenChange,
  onCreate,
}: {
  open: boolean;
  groupPath: string;
  source: GitRepositorySource;
  busy: boolean;
  onOpenChange: (open: boolean) => void;
  onCreate: (body: { provider: GitProviderKind; external_hook_id: string; ownership: GitWebhookOwnership; signing_secret?: string }) => Promise<void>;
}) {
  const { t } = useTranslation();
  const [formVisible, setFormVisible] = useState(false);
  const registrationQuery = useQuery({
    queryKey: ["git-webhook", groupPath, source.repository_key],
    queryFn: ({ signal }) => apiClient.getGitRepositoryWebhook(groupPath, source.repository_key, { signal }),
    enabled: open,
    retry: false,
  });
  const registration = registrationQuery.data ?? null;
  const missing = registrationQuery.error instanceof ApiError && registrationQuery.error.status === 404;
  const form = useForm({
    defaultValues: {
      externalHookId: "",
      ownership: "integration",
      signingSecret: "",
    } as WebhookFormValues,
    onSubmit: async ({ value }) => {
      await onCreate({
        provider: source.provider,
        external_hook_id: value.externalHookId.trim(),
        ownership: value.ownership,
        signing_secret: value.signingSecret.trim() || undefined,
      });
    },
  });

  const hookId = registration?.external_hook_id ?? "{external_hook_id}";
  const endpoint = resolveApiUrl(`/v1/webhooks/${source.provider}/${encodeURIComponent(hookId)}`);

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-h-[85vh] overflow-y-auto sm:max-w-2xl">
        <DialogHeader>
          <DialogTitle>{t("groups.gitSources.webhookTitle", { name: source.name })}</DialogTitle>
          <DialogDescription>{t("groups.gitSources.webhookDescription")}</DialogDescription>
        </DialogHeader>
        {registrationQuery.isLoading ? (
          <p className="text-sm text-muted-foreground">{t("common.loading")}</p>
        ) : null}
        {registrationQuery.error && !missing ? (
          <div role="alert" className="rounded-md border border-destructive/40 p-3 text-sm">
            {registrationQuery.error instanceof Error ? registrationQuery.error.message : t("groups.gitSources.webhookLoadFailed")}
            <Button type="button" size="sm" variant="outline" className="ml-2" onClick={() => { void registrationQuery.refetch(); }}>
              {t("common.retry")}
            </Button>
          </div>
        ) : null}
        {registration ? (
          <div className="grid gap-2 rounded-lg border p-3 text-sm" data-testid="git-webhook-registration">
            <div className="flex flex-wrap items-center gap-2">
              <Badge variant="outline">{registration.provider}</Badge>
              <Badge variant={registration.active ? "default" : "secondary"}>
                {registration.active ? t("groups.gitSources.webhookActive") : t("groups.gitSources.webhookInactive")}
              </Badge>
              <span className="font-mono text-xs">{registration.external_hook_id}</span>
            </div>
            <p className="text-xs text-muted-foreground">
              {t("groups.gitSources.webhookOwnership")}: {registration.ownership}
              {" · "}
              {registration.has_signing_secret
                ? t("groups.gitSources.webhookSecretPresent")
                : t("groups.gitSources.webhookSecretAbsent")}
            </p>
          </div>
        ) : null}
        {missing && !formVisible ? (
          <div className="grid gap-2">
            <p className="text-sm text-muted-foreground">{t("groups.gitSources.webhookMissing")}</p>
            <Button type="button" className="justify-self-start" onClick={() => setFormVisible(true)}>
              {t("groups.gitSources.registerWebhook")}
            </Button>
          </div>
        ) : null}
        {(missing && formVisible) ? (
          <form
            className="grid gap-3"
            onSubmit={(event) => {
              event.preventDefault();
              event.stopPropagation();
              void form.handleSubmit();
            }}
          >
            <form.Field
              name="externalHookId"
              validators={{ onChange: ({ value }) => required(value, t("groups.gitSources.required")) }}
            >
              {(field) => (
                <FormField
                  label={t("groups.gitSources.externalHookId")}
                  htmlFor="git-webhook-hook-id"
                  helper={t("groups.gitSources.externalHookIdHelper")}
                >
                  <Input
                    id="git-webhook-hook-id"
                    value={field.state.value}
                    aria-invalid={field.state.meta.errors.length > 0}
                    onChange={(event) => field.handleChange(event.target.value)}
                    onBlur={field.handleBlur}
                  />
                </FormField>
              )}
            </form.Field>
            <form.Field name="ownership">
              {(field) => (
                <FormField label={t("groups.gitSources.webhookOwnership")} htmlFor="git-webhook-ownership">
                  <Select
                    value={field.state.value}
                    onValueChange={(value) => field.handleChange(value as GitWebhookOwnership)}
                  >
                    <SelectTrigger id="git-webhook-ownership" className="w-full">
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      {WEBHOOK_OWNERSHIPS.map((ownership) => (
                        <SelectItem key={ownership} value={ownership}>{ownership}</SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                </FormField>
              )}
            </form.Field>
            <form.Field name="signingSecret">
              {(field) => (
                <FormField
                  label={t("groups.gitSources.signingSecret")}
                  htmlFor="git-webhook-secret"
                  helper={t("groups.gitSources.signingSecretHelper")}
                >
                  <Input
                    id="git-webhook-secret"
                    type="password"
                    autoComplete="new-password"
                    value={field.state.value}
                    onChange={(event) => field.handleChange(event.target.value)}
                    onBlur={field.handleBlur}
                  />
                </FormField>
              )}
            </form.Field>
            <form.Subscribe selector={(state) => [state.canSubmit, state.isSubmitting, state.values] as const}>
              {([canSubmit, isSubmitting, values]) => (
                <DialogFooter>
                  <Button
                    type="submit"
                    disabled={busy || isSubmitting || !canSubmit || !values.externalHookId.trim()}
                    aria-busy={busy || isSubmitting}
                  >
                    {t("groups.gitSources.registerWebhook")}
                  </Button>
                </DialogFooter>
              )}
            </form.Subscribe>
          </form>
        ) : null}

        <section className="grid gap-2 rounded-lg border p-3">
          <h3 className="text-sm font-medium">{t("groups.gitSources.webhookInstructions")}</h3>
          <dl className="grid gap-2 text-sm">
            <div className="grid gap-0.5">
              <dt className="text-xs font-medium text-muted-foreground uppercase">{t("groups.gitSources.webhookEndpoint")}</dt>
              <dd className="break-all font-mono text-xs">{endpoint}</dd>
            </div>
            <div className="grid gap-0.5">
              <dt className="text-xs font-medium text-muted-foreground uppercase">{t("groups.gitSources.webhookContentType")}</dt>
              <dd className="font-mono text-xs">application/json</dd>
            </div>
            <div className="grid gap-0.5">
              <dt className="text-xs font-medium text-muted-foreground uppercase">{t("groups.gitSources.webhookEvents")}</dt>
              <dd className="text-xs">{t("groups.gitSources.webhookEventsValue")}</dd>
            </div>
          </dl>
          <p className="text-xs text-muted-foreground">
            {source.provider === "forgejo"
              ? t("groups.gitSources.webhookForgejoInstructions")
              : t("groups.gitSources.webhookGithubInstructions")}
          </p>
        </section>
        <DialogFooter>
          <Button type="button" variant="outline" onClick={() => onOpenChange(false)}>
            {t("common.close")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
