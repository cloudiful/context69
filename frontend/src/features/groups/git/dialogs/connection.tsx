import { useForm } from "@tanstack/react-form";
import { useTranslation } from "react-i18next";

import { FormField } from "@/components/common/form-field";
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
import type {
  GitConnectionMode,
  GitProviderConnection,
  GitProviderConnectionRequest,
  GitProviderKind,
} from "@/services/api";

import { PROVIDERS, required } from "./fields";

const CONNECTION_MODES: GitConnectionMode[] = ["public", "token"];

type ConnectionFormValues = {
  connectionKey: string;
  provider: GitProviderKind;
  mode: GitConnectionMode;
  displayName: string;
  baseUrl: string;
  readCredential: string;
};

export function ConnectionDialog({
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
