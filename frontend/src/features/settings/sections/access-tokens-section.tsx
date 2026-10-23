import { useTranslation } from "react-i18next";

import { ConfirmDialog } from "@/components/common/confirm-dialog";
import { TablePagination } from "@/components/common/table-pagination";
import { FormField } from "@/components/common/form-field";
import { SettingsBlock, SettingsSection } from "@/components/common/settings-section";
import { TextField } from "@/components/common/text-field";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";
import { Textarea } from "@/components/ui/textarea";
import { SettingsGate } from "@/features/settings/settings-gate";
import { useSettings } from "@/features/settings/settings-context";
import {
  PERSONAL_ACCESS_TOKEN_EXPIRY_DAYS,
  PERSONAL_ACCESS_TOKEN_SCOPES,
} from "@/features/settings/use-personal-access-tokens";
import { formatTimestamp } from "@/lib/format";
import type { PersonalAccessTokenResponse, PersonalAccessTokenScope } from "@/services/api";

const SCOPE_HELPER_KEYS: Record<PersonalAccessTokenScope, string> = {
  search: "settings.personalAccessTokens.scopeSearchHelper",
  workspace: "settings.personalAccessTokens.scopeWorkspaceHelper",
  library: "settings.personalAccessTokens.scopeLibraryHelper",
  sources: "settings.personalAccessTokens.scopeSourcesHelper",
  settings: "settings.personalAccessTokens.scopeSettingsHelper",
  admin: "settings.personalAccessTokens.scopeAdminHelper",
};

function statusOf(token: PersonalAccessTokenResponse) {
  if (token.revoked_at) {
    return { labelKey: "settings.personalAccessTokens.statusRevoked", variant: "destructive" as const };
  }
  if (new Date(token.expires_at).getTime() <= Date.now()) {
    return { labelKey: "settings.personalAccessTokens.statusExpired", variant: "secondary" as const };
  }
  return { labelKey: "settings.personalAccessTokens.statusActive", variant: "outline" as const };
}

export function AccessTokensSection() {
  const { t } = useTranslation();
  const { tokens } = useSettings();
  const { draft, updateDraft } = tokens;

  function toggleScope(scope: PersonalAccessTokenScope, checked: boolean) {
    const next = checked
      ? [...draft.scopes, scope]
      : draft.scopes.filter((current) => current !== scope);
    updateDraft({ scopes: next });
  }

  return (
    <SettingsSection title={t("settings.personalAccessTokens.title")}>
      <SettingsGate>
      <div className="grid gap-3">
        <SettingsBlock compact>
          <div className="grid gap-3">
            <div className="grid max-w-2xl gap-y-2">
              {PERSONAL_ACCESS_TOKEN_SCOPES.map((scope) => (
                <div key={scope} className="flex items-start gap-2">
                  <Checkbox
                    id={`pat-scope-${scope}`}
                    data-testid={`pat-scope-${scope}`}
                    checked={draft.scopes.includes(scope)}
                    onCheckedChange={(checked) => toggleScope(scope, checked === true)}
                  />
                  <div className="grid gap-0.5">
                    <label htmlFor={`pat-scope-${scope}`} className="text-sm leading-tight">
                      {t(`settings.personalAccessTokens.scopeLabel.${scope}`)}
                    </label>
                    <p className="text-xs text-muted-foreground">{t(SCOPE_HELPER_KEYS[scope])}</p>
                  </div>
                </div>
              ))}
            </div>

            <div className="grid gap-2 md:grid-cols-[minmax(0,1fr)_11rem_auto] md:items-start">
              <TextField
                id="personal-access-token-name"
                testId="personal-access-token-name"
                label={t("settings.personalAccessTokens.name")}
                value={draft.name}
                onChange={(value) => updateDraft({ name: value })}
              />
              <FormField label={t("settings.personalAccessTokens.expiry")} htmlFor="personal-access-token-expiry">
                <Select
                  value={String(draft.expires_in_days)}
                  onValueChange={(value) => updateDraft({ expires_in_days: Number(value) })}
                >
                  <SelectTrigger
                    id="personal-access-token-expiry"
                    data-testid="personal-access-token-expiry"
                    aria-label={t("settings.personalAccessTokens.expiry")}
                  >
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    {PERSONAL_ACCESS_TOKEN_EXPIRY_DAYS.map((days) => (
                      <SelectItem key={days} value={String(days)}>
                        {t(`settings.personalAccessTokens.expiry${days}Days`)}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </FormField>
              <div className="flex items-end">
                <Button
                  type="button"
                  data-testid="personal-access-token-create"
                  disabled={tokens.creating || !!tokens.validationError}
                  aria-busy={tokens.creating}
                  onClick={() => void tokens.create()}
                >
                  {t("settings.personalAccessTokens.createAction")}
                </Button>
              </div>
            </div>
          </div>
        </SettingsBlock>

        {tokens.reveal ? (
          <SettingsBlock compact title={t("settings.personalAccessTokens.revealTitle")}>
            <div className="grid gap-3">
              <Alert>
                <AlertDescription>{t("settings.personalAccessTokens.revealWarning")}</AlertDescription>
              </Alert>
              <Textarea
                data-testid="personal-access-token-secret"
                value={tokens.reveal.access_token}
                readOnly
                rows={2}
                aria-label={t("settings.personalAccessTokens.revealTitle")}
                className="font-mono text-xs"
              />
              <div className="flex flex-wrap items-center gap-2">
                <Button type="button" variant="outline" onClick={() => void tokens.copy()}>
                  {t("settings.personalAccessTokens.copyAction")}
                </Button>
                <Button type="button" variant="outline" onClick={tokens.dismissReveal}>
                  {t("common.close")}
                </Button>
              </div>
            </div>
          </SettingsBlock>
        ) : null}

        <SettingsBlock compact>
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>{t("settings.personalAccessTokens.name")}</TableHead>
                <TableHead>{t("settings.personalAccessTokens.scopes")}</TableHead>
                <TableHead>{t("settings.personalAccessTokens.createdAt")}</TableHead>
                <TableHead>{t("settings.personalAccessTokens.expiresAt")}</TableHead>
                <TableHead>{t("settings.personalAccessTokens.lastUsedAt")}</TableHead>
                <TableHead>{t("settings.personalAccessTokens.status")}</TableHead>
                <TableHead>{t("settings.personalAccessTokens.actions")}</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {tokens.items.length === 0 ? (
                <TableRow>
                  <TableCell colSpan={7} className="text-sm text-muted-foreground">
                    {tokens.loading
                      ? t("common.loading")
                      : t("settings.personalAccessTokens.empty")}
                  </TableCell>
                </TableRow>
              ) : (
                tokens.items.map((token) => {
                  const status = statusOf(token);
                  return (
                    <TableRow key={token.token_id}>
                      <TableCell>{token.name}</TableCell>
                      <TableCell>
                        {token.scopes
                          .map((scope) => t(`settings.personalAccessTokens.scopeLabel.${scope}`))
                          .join(", ")}
                      </TableCell>
                      <TableCell className="whitespace-nowrap">{formatTimestamp(token.created_at)}</TableCell>
                      <TableCell className="whitespace-nowrap">{formatTimestamp(token.expires_at)}</TableCell>
                      <TableCell className="whitespace-nowrap">
                        {token.last_used_at
                          ? formatTimestamp(token.last_used_at)
                          : t("settings.personalAccessTokens.neverUsed")}
                      </TableCell>
                      <TableCell>
                        <Badge variant={status.variant} className="whitespace-nowrap">
                          {t(status.labelKey)}
                        </Badge>
                      </TableCell>
                      <TableCell>
                        <Button
                          type="button"
                          variant="outline"
                          size="sm"
                          disabled={!!token.revoked_at}
                          onClick={() => tokens.setRevokeTarget(token)}
                        >
                          {t("settings.personalAccessTokens.revokeAction")}
                        </Button>
                      </TableCell>
                    </TableRow>
                  );
                })
              )}
            </TableBody>
          </Table>
          <TablePagination
            pagination={tokens.pagination}
            onPage={tokens.changePage}
            onPageSize={tokens.changePageSize}
          />
        </SettingsBlock>
      </div>
      </SettingsGate>

      <ConfirmDialog
        open={tokens.revokeTarget !== null}
        title={t("settings.personalAccessTokens.revokeAction")}
        message={t("settings.personalAccessTokens.revokeConfirm", { name: tokens.revokeTarget?.name ?? "" })}
        acceptLabel={t("settings.personalAccessTokens.revokeAction")}
        onAccept={() => {
          const target = tokens.revokeTarget;
          tokens.setRevokeTarget(null);
          if (target) {
            void tokens.revoke(target);
          }
        }}
        onCancel={() => tokens.setRevokeTarget(null)}
      />
    </SettingsSection>
  );
}
