import { useForm } from "@tanstack/react-form";
import { useQuery } from "@tanstack/react-query";
import { useState } from "react";
import { useTranslation } from "react-i18next";

import { FormField } from "@/components/common/form-field";
import { ToggleField } from "@/components/common/toggle-field";
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
import { Textarea } from "@/components/ui/textarea";
import {
  ApiError,
  apiClient,
  type GitConnectionMode,
  type GitIndexProfile,
  type GitPinnedCommitPatch,
  type GitProviderConnection,
  type GitProviderConnectionRequest,
  type GitProviderKind,
  type GitRepositoryRegistrationRequest,
  type GitRepositorySettingsUpdateRequest,
  type GitRepositorySource,
  type GitWebhookOwnership,
} from "@/services/api";
import { resolveApiUrl } from "@/lib/api-base";

const NONE = "__none__";
const PROVIDERS: GitProviderKind[] = ["github", "forgejo"];
const CONNECTION_MODES: GitConnectionMode[] = ["public", "token"];
const INDEX_PROFILES: GitIndexProfile[] = ["lexical", "hybrid", "full_semantic"];
const WEBHOOK_OWNERSHIPS: GitWebhookOwnership[] = ["integration", "external", "unknown"];

function splitPatterns(value: string): string[] {
  return value
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter((line) => line.length > 0);
}

function optionalNumber(value: string): number | null {
  const trimmed = value.trim();
  if (!trimmed) {
    return null;
  }
  const parsed = Number(trimmed);
  return Number.isFinite(parsed) ? parsed : Number.NaN;
}

type SourceFormValues = {
  provider: GitProviderKind;
  canonicalUrl: string;
  defaultBranch: string;
  targetRef: string;
  pinnedCommit: string;
  connectionKey: string;
  indexProfile: GitIndexProfile;
  includePatterns: string;
  excludePatterns: string;
  intervalSeconds: string;
  webhookEnabled: boolean;
  enabled: boolean;
  retentionCount: string;
};

function sourceFormDefaults(source: GitRepositorySource | null): SourceFormValues {
  return {
    provider: source?.provider ?? "github",
    canonicalUrl: source?.canonical_url ?? "",
    defaultBranch: source?.default_branch ?? "main",
    targetRef: source?.version.ref_name ?? "refs/heads/main",
    pinnedCommit: source?.version.commit_sha ?? "",
    connectionKey: source?.connection_key ?? NONE,
    indexProfile: source?.index_profile ?? "lexical",
    includePatterns: (source?.include_patterns ?? []).join("\n"),
    excludePatterns: (source?.exclude_patterns ?? []).join("\n"),
    intervalSeconds: source?.interval_seconds ? String(source.interval_seconds) : "",
    webhookEnabled: source?.webhook_enabled ?? false,
    enabled: source?.enabled ?? true,
    retentionCount: String(source?.retention_count ?? 2),
  };
}

function pinnedCommitPatch(current: string | null | undefined, value: string): GitPinnedCommitPatch {
  const trimmed = value.trim();
  const existing = current ?? "";
  if (trimmed === existing) {
    return { op: "keep" };
  }
  return trimmed ? { op: "set", value: trimmed } : { op: "clear" };
}

function required(value: string, message: string): string | undefined {
  return value.trim().length > 0 ? undefined : message;
}

export function GitSourceDialog({
  open,
  source,
  connections,
  busy,
  onOpenChange,
  onCreate,
  onUpdate,
}: {
  open: boolean;
  source: GitRepositorySource | null;
  connections: GitProviderConnection[];
  busy: boolean;
  onOpenChange: (open: boolean) => void;
  onCreate: (body: GitRepositoryRegistrationRequest) => Promise<void>;
  onUpdate: (body: GitRepositorySettingsUpdateRequest) => Promise<void>;
}) {
  const { t } = useTranslation();
  const editing = source !== null;
  const form = useForm({
    defaultValues: sourceFormDefaults(source),
    onSubmit: async ({ value }) => {
      if (editing) {
        await onUpdate({
          target_ref: value.targetRef.trim(),
          pinned_commit: pinnedCommitPatch(source?.version.commit_sha, value.pinnedCommit),
          index_profile: value.indexProfile,
          include_patterns: splitPatterns(value.includePatterns),
          exclude_patterns: splitPatterns(value.excludePatterns),
          interval_seconds: optionalNumber(value.intervalSeconds),
          webhook_enabled: value.webhookEnabled,
          enabled: value.enabled,
          retention_count: Number(value.retentionCount),
        });
        return;
      }
      await onCreate({
        provider: value.provider,
        canonical_url: value.canonicalUrl.trim(),
        default_branch: value.defaultBranch.trim(),
        target_ref: value.targetRef.trim(),
        pinned_commit: value.pinnedCommit.trim() || undefined,
        connection_key: value.connectionKey === NONE ? undefined : value.connectionKey,
        index_profile: value.indexProfile,
        include_patterns: splitPatterns(value.includePatterns),
        exclude_patterns: splitPatterns(value.excludePatterns),
        interval_seconds: optionalNumber(value.intervalSeconds),
        webhook_enabled: value.webhookEnabled,
        enabled: value.enabled,
        retention_count: Number(value.retentionCount),
      });
    },
  });

  const key = source?.repository_key ?? "new";

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-h-[85vh] overflow-y-auto sm:max-w-2xl">
        <DialogHeader>
          <DialogTitle>
            {editing ? t("groups.gitSources.editTitle") : t("groups.gitSources.createTitle")}
          </DialogTitle>
          <DialogDescription>
            {editing ? t("groups.gitSources.editDescription") : t("groups.gitSources.createDescription")}
          </DialogDescription>
        </DialogHeader>
        <form
          key={key}
          className="grid gap-3"
          onSubmit={(event) => {
            event.preventDefault();
            event.stopPropagation();
            void form.handleSubmit();
          }}
        >
          {editing ? null : (
            <>
              <form.Field
                name="provider"
                validators={{ onChange: ({ value }) => (value ? undefined : t("groups.gitSources.required")) }}
              >
                {(field) => (
                  <FormField label={t("groups.gitSources.provider")} htmlFor="git-source-provider">
                    <Select
                      value={field.state.value}
                      onValueChange={(value) => field.handleChange(value as GitProviderKind)}
                    >
                      <SelectTrigger id="git-source-provider" className="w-full">
                        <SelectValue />
                      </SelectTrigger>
                      <SelectContent>
                        {PROVIDERS.map((provider) => (
                          <SelectItem key={provider} value={provider}>{provider}</SelectItem>
                        ))}
                      </SelectContent>
                    </Select>
                  </FormField>
                )}
              </form.Field>

              <form.Field
                name="canonicalUrl"
                validators={{
                  onChange: ({ value }) => required(value, t("groups.gitSources.required")),
                }}
              >
                {(field) => (
                  <FormField
                    label={t("groups.gitSources.canonicalUrl")}
                    htmlFor="git-source-url"
                    helper={t("groups.gitSources.canonicalUrlHelper")}
                  >
                    <Input
                      id="git-source-url"
                      value={field.state.value}
                      aria-invalid={field.state.meta.errors.length > 0}
                      onChange={(event) => field.handleChange(event.target.value)}
                      onBlur={field.handleBlur}
                    />
                  </FormField>
                )}
              </form.Field>

              <div className="grid gap-3 sm:grid-cols-2">
                <form.Field
                  name="defaultBranch"
                  validators={{ onChange: ({ value }) => required(value, t("groups.gitSources.required")) }}
                >
                  {(field) => (
                    <FormField label={t("groups.gitSources.defaultBranch")} htmlFor="git-source-branch">
                      <Input
                        id="git-source-branch"
                        value={field.state.value}
                        onChange={(event) => field.handleChange(event.target.value)}
                        onBlur={field.handleBlur}
                      />
                    </FormField>
                  )}
                </form.Field>
                <form.Subscribe selector={(state) => state.values.provider}>
                  {(provider) => (
                    <form.Field name="connectionKey">
                      {(field) => (
                        <FormField
                          label={t("groups.gitSources.connection")}
                          htmlFor="git-source-connection"
                          helper={t("groups.gitSources.connectionHelper")}
                        >
                          <Select value={field.state.value} onValueChange={field.handleChange}>
                            <SelectTrigger id="git-source-connection" className="w-full">
                              <SelectValue />
                            </SelectTrigger>
                            <SelectContent>
                              <SelectItem value={NONE}>{t("groups.gitSources.noConnection")}</SelectItem>
                              {connections
                                .filter((connection) => connection.provider === provider)
                                .map((connection) => (
                                  <SelectItem key={connection.connection_key} value={connection.connection_key}>
                                    {connection.display_name}
                                  </SelectItem>
                                ))}
                            </SelectContent>
                          </Select>
                        </FormField>
                      )}
                    </form.Field>
                  )}
                </form.Subscribe>
              </div>
            </>
          )}

          <div className="grid gap-3 sm:grid-cols-2">
            <form.Field
              name="targetRef"
              validators={{ onChange: ({ value }) => required(value, t("groups.gitSources.required")) }}
            >
              {(field) => (
                <FormField
                  label={t("groups.gitSources.targetRef")}
                  htmlFor="git-source-ref"
                  helper={t("groups.gitSources.targetRefHelper")}
                >
                  <Input
                    id="git-source-ref"
                    value={field.state.value}
                    onChange={(event) => field.handleChange(event.target.value)}
                    onBlur={field.handleBlur}
                  />
                </FormField>
              )}
            </form.Field>
            <form.Field name="pinnedCommit">
              {(field) => (
                <FormField
                  label={t("groups.gitSources.pinnedCommit")}
                  htmlFor="git-source-commit"
                  helper={t("groups.gitSources.pinnedCommitHelper")}
                >
                  <Input
                    id="git-source-commit"
                    value={field.state.value}
                    onChange={(event) => field.handleChange(event.target.value)}
                    onBlur={field.handleBlur}
                  />
                </FormField>
              )}
            </form.Field>
          </div>

          <form.Field name="indexProfile">
            {(field) => (
              <FormField
                label={t("groups.gitSources.indexProfile")}
                htmlFor="git-source-profile"
                helper={t("groups.gitSources.indexProfileHelper")}
              >
                <Select
                  value={field.state.value}
                  onValueChange={(value) => field.handleChange(value as GitIndexProfile)}
                >
                  <SelectTrigger id="git-source-profile" className="w-full">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    {INDEX_PROFILES.map((profile) => (
                      <SelectItem key={profile} value={profile}>{profile}</SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </FormField>
            )}
          </form.Field>

          <div className="grid gap-3 sm:grid-cols-2">
            <form.Field name="includePatterns">
              {(field) => (
                <FormField
                  label={t("groups.gitSources.includePatterns")}
                  htmlFor="git-source-include"
                  helper={t("groups.gitSources.patternsHelper")}
                >
                  <Textarea
                    id="git-source-include"
                    rows={4}
                    value={field.state.value}
                    onChange={(event) => field.handleChange(event.target.value)}
                    onBlur={field.handleBlur}
                  />
                </FormField>
              )}
            </form.Field>
            <form.Field name="excludePatterns">
              {(field) => (
                <FormField
                  label={t("groups.gitSources.excludePatterns")}
                  htmlFor="git-source-exclude"
                  helper={t("groups.gitSources.excludeHelper")}
                >
                  <Textarea
                    id="git-source-exclude"
                    rows={4}
                    value={field.state.value}
                    onChange={(event) => field.handleChange(event.target.value)}
                    onBlur={field.handleBlur}
                  />
                </FormField>
              )}
            </form.Field>
          </div>

          <div className="grid gap-3 sm:grid-cols-2">
            <form.Field
              name="intervalSeconds"
              validators={{
                onChange: ({ value }) => {
                  if (!value.trim()) {
                    return undefined;
                  }
                  const parsed = optionalNumber(value);
                  return parsed !== null && Number.isFinite(parsed) && parsed >= 300 && parsed <= 2_592_000
                    ? undefined
                    : t("groups.gitSources.intervalInvalid");
                },
              }}
            >
              {(field) => (
                <FormField
                  label={t("groups.gitSources.intervalSeconds")}
                  htmlFor="git-source-interval"
                  helper={t("groups.gitSources.intervalHelper")}
                >
                  <Input
                    id="git-source-interval"
                    type="number"
                    min={300}
                    max={2592000}
                    value={field.state.value}
                    aria-invalid={field.state.meta.errors.length > 0}
                    onChange={(event) => field.handleChange(event.target.value)}
                    onBlur={field.handleBlur}
                  />
                </FormField>
              )}
            </form.Field>
            <form.Field
              name="retentionCount"
              validators={{
                onChange: ({ value }) => {
                  const parsed = Number(value);
                  return Number.isInteger(parsed) && parsed >= 1 && parsed <= 50
                    ? undefined
                    : t("groups.gitSources.retentionInvalid");
                },
              }}
            >
              {(field) => (
                <FormField
                  label={t("groups.gitSources.retentionCount")}
                  htmlFor="git-source-retention"
                  helper={t("groups.gitSources.retentionHelper")}
                >
                  <Input
                    id="git-source-retention"
                    type="number"
                    min={1}
                    max={50}
                    value={field.state.value}
                    aria-invalid={field.state.meta.errors.length > 0}
                    onChange={(event) => field.handleChange(event.target.value)}
                    onBlur={field.handleBlur}
                  />
                </FormField>
              )}
            </form.Field>
          </div>

          <div className="grid gap-2 sm:grid-cols-2">
            <form.Field name="webhookEnabled">
              {(field) => (
                <ToggleField
                  id="git-source-webhook-enabled"
                  label={t("groups.gitSources.webhookEnabled")}
                  helper={t("groups.gitSources.webhookEnabledHelper")}
                  checked={field.state.value}
                  onCheckedChange={field.handleChange}
                />
              )}
            </form.Field>
            <form.Field name="enabled">
              {(field) => (
                <ToggleField
                  id="git-source-enabled"
                  label={t("groups.gitSources.sourceEnabled")}
                  helper={t("groups.gitSources.sourceEnabledHelper")}
                  checked={field.state.value}
                  onCheckedChange={field.handleChange}
                />
              )}
            </form.Field>
          </div>

          <form.Subscribe selector={(state) => [state.canSubmit, state.isSubmitting, state.values] as const}>
            {([canSubmit, isSubmitting, values]) => {
              const forgejoMissingConnection = !editing
                && values.provider === "forgejo"
                && values.connectionKey === NONE;
              const createMissing = !editing
                && (!values.canonicalUrl.trim() || !values.defaultBranch.trim() || !values.targetRef.trim());
              return (
                <DialogFooter>
                  <Button type="button" variant="outline" onClick={() => onOpenChange(false)}>
                    {t("common.cancel")}
                  </Button>
                  <Button
                    type="submit"
                    disabled={busy || isSubmitting || !canSubmit || forgejoMissingConnection || createMissing}
                    aria-busy={busy || isSubmitting}
                  >
                    {editing ? t("common.save") : t("groups.gitSources.create")}
                  </Button>
                </DialogFooter>
              );
            }}
          </form.Subscribe>
        </form>
      </DialogContent>
    </Dialog>
  );
}

type ConnectionFormValues = {
  connectionKey: string;
  provider: GitProviderKind;
  mode: GitConnectionMode;
  displayName: string;
  baseUrl: string;
  readCredential: string;
};

export function GitConnectionDialog({
  open,
  connections,
  busy,
  onOpenChange,
  onCreate,
}: {
  open: boolean;
  connections: GitProviderConnection[];
  busy: boolean;
  onOpenChange: (open: boolean) => void;
  onCreate: (connectionKey: string, body: GitProviderConnectionRequest) => Promise<void>;
}) {
  const { t } = useTranslation();
  const form = useForm({
    defaultValues: {
      connectionKey: "",
      provider: "forgejo",
      mode: "token",
      displayName: "",
      baseUrl: "",
      readCredential: "",
    } as ConnectionFormValues,
    onSubmit: async ({ value }) => {
      await onCreate(value.connectionKey.trim(), {
        provider: value.provider,
        mode: value.mode,
        display_name: value.displayName.trim(),
        base_url: value.baseUrl.trim(),
        read_credential: value.readCredential.trim()
          ? { op: "set", value: value.readCredential }
          : { op: "keep" },
      });
    },
  });

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-h-[85vh] overflow-y-auto">
        <DialogHeader>
          <DialogTitle>{t("groups.gitSources.createConnectionTitle")}</DialogTitle>
          <DialogDescription>{t("groups.gitSources.createConnectionDescription")}</DialogDescription>
        </DialogHeader>
        <form
          className="grid gap-3"
          onSubmit={(event) => {
            event.preventDefault();
            event.stopPropagation();
            void form.handleSubmit();
          }}
        >
          <form.Field
            name="connectionKey"
            validators={{
              onChange: ({ value }) => {
                const trimmed = value.trim();
                if (!trimmed) {
                  return t("groups.gitSources.required");
                }
                return /^[A-Za-z0-9._-]+$/.test(trimmed) && !trimmed.startsWith(".")
                  ? undefined
                  : t("groups.gitSources.connectionKeyInvalid");
              },
            }}
          >
            {(field) => (
              <FormField
                label={t("groups.gitSources.connectionKey")}
                htmlFor="git-connection-key"
                helper={t("groups.gitSources.connectionKeyHelper")}
              >
                <Input
                  id="git-connection-key"
                  value={field.state.value}
                  aria-invalid={field.state.meta.errors.length > 0}
                  onChange={(event) => field.handleChange(event.target.value)}
                  onBlur={field.handleBlur}
                />
              </FormField>
            )}
          </form.Field>
          <div className="grid gap-3 sm:grid-cols-2">
            <form.Field name="provider">
              {(field) => (
                <FormField label={t("groups.gitSources.provider")} htmlFor="git-connection-provider">
                  <Select
                    value={field.state.value}
                    onValueChange={(value) => field.handleChange(value as GitProviderKind)}
                  >
                    <SelectTrigger id="git-connection-provider" className="w-full">
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      {PROVIDERS.map((provider) => (
                        <SelectItem key={provider} value={provider}>{provider}</SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                </FormField>
              )}
            </form.Field>
            <form.Field name="mode">
              {(field) => (
                <FormField label={t("groups.gitSources.connectionMode")} htmlFor="git-connection-mode">
                  <Select
                    value={field.state.value}
                    onValueChange={(value) => field.handleChange(value as GitConnectionMode)}
                  >
                    <SelectTrigger id="git-connection-mode" className="w-full">
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      {CONNECTION_MODES.map((mode) => (
                        <SelectItem key={mode} value={mode}>{mode}</SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                </FormField>
              )}
            </form.Field>
          </div>
          <form.Field
            name="displayName"
            validators={{ onChange: ({ value }) => required(value, t("groups.gitSources.required")) }}
          >
            {(field) => (
              <FormField label={t("groups.gitSources.connectionName")} htmlFor="git-connection-name">
                <Input
                  id="git-connection-name"
                  value={field.state.value}
                  onChange={(event) => field.handleChange(event.target.value)}
                  onBlur={field.handleBlur}
                />
              </FormField>
            )}
          </form.Field>
          <form.Field
            name="baseUrl"
            validators={{
              onChange: ({ value }) => {
                if (!value.trim()) {
                  return t("groups.gitSources.required");
                }
                return /^https?:\/\//.test(value.trim())
                  ? undefined
                  : t("groups.gitSources.connectionBaseUrlInvalid");
              },
            }}
          >
            {(field) => (
              <FormField
                label={t("groups.gitSources.connectionBaseUrl")}
                htmlFor="git-connection-base-url"
                helper={t("groups.gitSources.connectionBaseUrlHelper")}
              >
                <Input
                  id="git-connection-base-url"
                  value={field.state.value}
                  aria-invalid={field.state.meta.errors.length > 0}
                  onChange={(event) => field.handleChange(event.target.value)}
                  onBlur={field.handleBlur}
                />
              </FormField>
            )}
          </form.Field>
          <form.Field name="readCredential">
            {(field) => (
              <FormField
                label={t("groups.gitSources.readCredential")}
                htmlFor="git-connection-credential"
                helper={t("groups.gitSources.readCredentialHelper")}
              >
                <Input
                  id="git-connection-credential"
                  type="password"
                  autoComplete="new-password"
                  value={field.state.value}
                  onChange={(event) => field.handleChange(event.target.value)}
                  onBlur={field.handleBlur}
                />
              </FormField>
            )}
          </form.Field>
          <p className="text-xs text-muted-foreground">{t("groups.gitSources.connectionCredentialNotice")}</p>
          {connections.length > 0 ? (
            <p className="text-xs text-muted-foreground">{t("groups.gitSources.connectionExistingNotice")}</p>
          ) : null}
          <form.Subscribe selector={(state) => [state.canSubmit, state.isSubmitting, state.values] as const}>
            {([canSubmit, isSubmitting, values]) => {
              const missing = !values.connectionKey.trim() || !values.displayName.trim() || !values.baseUrl.trim();
              return (
                <DialogFooter>
                  <Button type="button" variant="outline" onClick={() => onOpenChange(false)}>
                    {t("common.cancel")}
                  </Button>
                  <Button
                    type="submit"
                    disabled={busy || isSubmitting || !canSubmit || missing}
                    aria-busy={busy || isSubmitting}
                  >
                    {t("groups.gitSources.createConnection")}
                  </Button>
                </DialogFooter>
              );
            }}
          </form.Subscribe>
        </form>
      </DialogContent>
    </Dialog>
  );
}

type WebhookFormValues = {
  externalHookId: string;
  ownership: GitWebhookOwnership;
  signingSecret: string;
};

export function GitWebhookDialog({
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
