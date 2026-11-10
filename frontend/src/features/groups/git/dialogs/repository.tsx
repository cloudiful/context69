import { useForm } from "@tanstack/react-form";
import { useTranslation } from "react-i18next";

import { FormField } from "@/components/common/form-field";
import { ToggleField } from "@/components/common/toggle-field";
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
import type {
  GitIndexProfile,
  GitPinnedCommitPatch,
  GitProviderConnection,
  GitProviderKind,
  GitRepositoryRegistrationRequest,
  GitRepositorySettingsUpdateRequest,
  GitRepositorySource,
} from "@/services/api";

import { PROVIDERS, required } from "./fields";

const NONE = "__none__";
const INDEX_PROFILES: GitIndexProfile[] = ["lexical", "hybrid", "full_semantic"];

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

export function RepositoryDialog({
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
