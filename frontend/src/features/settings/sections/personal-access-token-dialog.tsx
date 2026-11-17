import { Info } from "lucide-react";
import { useEffect, useId, useState } from "react";
import { useTranslation } from "react-i18next";

import { FormField } from "@/components/common/form-field";
import { TextField } from "@/components/common/text-field";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { useAuthSession } from "@/hooks/use-auth";
import {
  DEFAULT_EXPIRY_DAYS,
  PERSONAL_ACCESS_TOKEN_EXPIRY_DAYS,
  PERSONAL_ACCESS_TOKEN_SCOPES,
} from "@/features/settings/use-personal-access-tokens";
import { formatTimestamp } from "@/lib/format";
import type { PersonalAccessTokenResponse, PersonalAccessTokenScope, TokenGroupScope } from "@/services/api";

import { PersonalAccessTokenGroupPicker } from "./personal-access-token-group-picker";

const SCOPE_HELPER_KEYS: Record<PersonalAccessTokenScope, string> = {
  search: "settings.personalAccessTokens.scopeSearchHelper",
  workspace: "settings.personalAccessTokens.scopeWorkspaceHelper",
  library: "settings.personalAccessTokens.scopeLibraryHelper",
  sources: "settings.personalAccessTokens.scopeSourcesHelper",
  settings: "settings.personalAccessTokens.scopeSettingsHelper",
  admin: "settings.personalAccessTokens.scopeAdminHelper",
};

export interface PersonalAccessTokenFormValue {
  name: string;
  scopes: PersonalAccessTokenScope[];
  groupScope: TokenGroupScope;
  expiresInDays: number;
}

/**
 * Create and edit dialog for one token. Create chooses the lifetime; edit
 * shows the fixed expiry and never pretends to change it.
 */
export function PersonalAccessTokenDialog({
  open,
  mode,
  token,
  busy,
  error,
  onOpenChange,
  onSubmit,
}: {
  open: boolean;
  mode: "create" | "edit";
  token: PersonalAccessTokenResponse | null;
  busy: boolean;
  error: string | null;
  onOpenChange: (open: boolean) => void;
  onSubmit: (value: PersonalAccessTokenFormValue) => void;
}) {
  const { t } = useTranslation();
  const { user } = useAuthSession();
  const groupModeName = useId();
  const [name, setName] = useState("");
  const [scopes, setScopes] = useState<PersonalAccessTokenScope[]>(["search"]);
  const [groupMode, setGroupMode] = useState<"all" | "groups">("groups");
  const [groupIds, setGroupIds] = useState<number[]>([]);
  const [expiresInDays, setExpiresInDays] = useState<number>(DEFAULT_EXPIRY_DAYS);

  useEffect(() => {
    if (!open) {
      return;
    }
    setName(token?.name ?? "");
    const initialScopes: PersonalAccessTokenScope[] = token ? [...token.scopes] : ["search"];
    setScopes(
      token?.group_scope.mode === "groups"
        ? initialScopes.filter((scope) => scope !== "settings" && scope !== "admin")
        : initialScopes,
    );
    setGroupMode(token ? token.group_scope.mode : "groups");
    setGroupIds(token?.group_scope.mode === "groups" ? [...token.group_scope.group_ids] : []);
    setExpiresInDays(DEFAULT_EXPIRY_DAYS);
  }, [open, token]);

  const isAdmin = user?.is_admin === true;
  const restrictedMode = groupMode === "groups";
  const expired = token ? new Date(token.expires_at).getTime() <= Date.now() : false;
  const revoked = token?.revoked_at != null;

  function toggleScope(scope: PersonalAccessTokenScope, checked: boolean) {
    if (checked) {
      if (!scopes.includes(scope)) {
        setScopes([...scopes, scope]);
      }
      return;
    }
    setScopes(scopes.filter((current) => current !== scope));
  }

  function chooseGroupMode(next: "all" | "groups") {
    setGroupMode(next);
    if (next === "groups") {
      setScopes((current) => current.filter((scope) => scope !== "settings" && scope !== "admin"));
    }
  }

  const missingGroup = restrictedMode && groupIds.length === 0;
  const blockedScope = restrictedMode && scopes.some((scope) => scope === "settings" || scope === "admin");
  const canSubmit = name.trim().length > 0
    && scopes.length > 0
    && !missingGroup
    && !blockedScope
    && !busy;

  function submit() {
    if (!canSubmit) {
      return;
    }
    onSubmit({
      name: name.trim(),
      scopes,
      groupScope: restrictedMode ? { mode: "groups", group_ids: groupIds } : { mode: "all" },
      expiresInDays,
    });
  }

  const groupSummary = restrictedMode
    ? t("settings.personalAccessTokens.summaryGroups", { count: groupIds.length })
    : t("settings.personalAccessTokens.summaryAllGroups");
  const scopeSummary = scopes.length > 0
    ? scopes.map((scope) => t(`settings.personalAccessTokens.scopeLabel.${scope}`)).join(", ")
    : t("settings.personalAccessTokens.scopesRequired");

  return (
    <Dialog open={open} onOpenChange={(next) => {
      if (!next && busy) {
        return;
      }
      onOpenChange(next);
    }}>
      <DialogContent className="max-h-[calc(100vh-2rem)] overflow-y-auto sm:max-w-4xl">
        <DialogHeader>
          <DialogTitle>
            {t(mode === "create"
              ? "settings.personalAccessTokens.createTitle"
              : "settings.personalAccessTokens.editTitle")}
          </DialogTitle>
          <DialogDescription>
            {t(mode === "create"
              ? "settings.personalAccessTokens.createDescription"
              : "settings.personalAccessTokens.editDescription")}
          </DialogDescription>
        </DialogHeader>

        <div className="grid gap-4">
          <div
            className={mode === "create"
              ? "grid gap-3 sm:grid-cols-[minmax(0,1fr)_12rem] sm:items-end"
              : "grid gap-1.5"}
          >
            <TextField
              id="personal-access-token-name"
              testId="personal-access-token-name"
              label={t("settings.personalAccessTokens.name")}
              value={name}
              onChange={setName}
              autoComplete="off"
            />
            {mode === "create" ? (
              <FormField
                label={t("settings.personalAccessTokens.expiry")}
                htmlFor="personal-access-token-expiry"
              >
                <Select
                  value={String(expiresInDays)}
                  onValueChange={(value) => setExpiresInDays(Number(value))}
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
            ) : (
              <div className="grid gap-0.5">
                <span className="text-sm font-medium">{t("settings.personalAccessTokens.expiry")}</span>
                <span className="text-sm">{formatTimestamp(token?.expires_at)}</span>
                <span className="text-xs text-muted-foreground">
                  {t("settings.personalAccessTokens.expiryFixed")}
                </span>
                {expired || revoked ? (
                  <span className="text-xs text-muted-foreground">
                    {t("settings.personalAccessTokens.editStatusNote")}
                  </span>
                ) : null}
              </div>
            )}
          </div>

          <div className="grid gap-4 lg:grid-cols-2">
            <section className="grid content-start gap-2" aria-label={t("settings.personalAccessTokens.scopes")}>
              <h3 className="text-sm font-medium">{t("settings.personalAccessTokens.scopes")}</h3>
              <div className="grid gap-2">
                {PERSONAL_ACCESS_TOKEN_SCOPES.map((scope) => {
                  const disabledByGroup = restrictedMode && (scope === "settings" || scope === "admin");
                  const disabledByAccount = scope === "admin" && !isAdmin && !scopes.includes("admin");
                  const disabled = disabledByGroup || disabledByAccount;
                  const label = t(`settings.personalAccessTokens.scopeLabel.${scope}`);
                  return (
                    <div key={scope} className="flex items-start gap-2">
                      <Checkbox
                        id={`pat-scope-${scope}`}
                        data-testid={`pat-scope-${scope}`}
                        className="mt-0.5"
                        checked={scopes.includes(scope)}
                        disabled={disabled}
                        onCheckedChange={(checked) => toggleScope(scope, checked === true)}
                      />
                      <div className="flex min-w-0 items-center gap-1">
                        <label
                          htmlFor={`pat-scope-${scope}`}
                          className={disabled ? "text-sm leading-tight text-muted-foreground" : "text-sm leading-tight"}
                        >
                          {label}
                        </label>
                        <Tooltip>
                          <TooltipTrigger asChild>
                            <button
                              type="button"
                              className="rounded-full p-0.5 text-muted-foreground outline-none hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring"
                              aria-label={t("settings.personalAccessTokens.scopeInfoLabel", { scope: label })}
                            >
                              <Info aria-hidden="true" className="size-3.5" />
                            </button>
                          </TooltipTrigger>
                          <TooltipContent>{t(SCOPE_HELPER_KEYS[scope])}</TooltipContent>
                        </Tooltip>
                      </div>
                    </div>
                  );
                })}
              </div>
              {!isAdmin ? (
                <p className="text-xs text-muted-foreground">
                  {t("settings.personalAccessTokens.adminScopeUnavailable")}
                </p>
              ) : null}
              {restrictedMode ? (
                <p className="text-xs text-muted-foreground">
                  {t("settings.personalAccessTokens.specificScopeBlocked")}
                </p>
              ) : null}
              {scopes.length === 0 ? (
                <p role="alert" className="text-xs text-destructive">
                  {t("settings.personalAccessTokens.scopesRequired")}
                </p>
              ) : null}
            </section>

            <section className="grid content-start gap-2" aria-label={t("settings.personalAccessTokens.groupScope")}>
              <h3 className="text-sm font-medium">{t("settings.personalAccessTokens.groupScope")}</h3>
              <fieldset className="grid gap-1.5">
                <legend className="sr-only">{t("settings.personalAccessTokens.groupScope")}</legend>
                <label className="flex items-start gap-2">
                  <input
                    type="radio"
                    name={groupModeName}
                    className="mt-1 accent-primary"
                    checked={groupMode === "all"}
                    onChange={() => chooseGroupMode("all")}
                  />
                  <span className="grid gap-0.5">
                    <span className="text-sm">{t("settings.personalAccessTokens.groupScopeAll")}</span>
                    <span className="text-xs text-muted-foreground">
                      {t("settings.personalAccessTokens.groupScopeAllHint")}
                    </span>
                  </span>
                </label>
                <label className="flex items-start gap-2">
                  <input
                    type="radio"
                    name={groupModeName}
                    className="mt-1 accent-primary"
                    checked={groupMode === "groups"}
                    onChange={() => chooseGroupMode("groups")}
                  />
                  <span className="grid gap-0.5">
                    <span className="text-sm">{t("settings.personalAccessTokens.groupScopeGroups")}</span>
                    <span className="text-xs text-muted-foreground">
                      {t("settings.personalAccessTokens.groupScopeGroupsHint")}
                    </span>
                  </span>
                </label>
              </fieldset>

              {restrictedMode ? (
                <PersonalAccessTokenGroupPicker
                  selectedIds={groupIds}
                  onSelectedIdsChange={setGroupIds}
                />
              ) : (
                <p className="rounded-md border bg-muted/40 p-2 text-xs text-muted-foreground">
                  {t("settings.personalAccessTokens.groupScopeAllHint")}
                </p>
              )}
              {missingGroup ? (
                <p role="alert" className="text-xs text-destructive">
                  {t("settings.personalAccessTokens.groupScopeRequired")}
                </p>
              ) : null}
            </section>
          </div>

          <div className="grid gap-1 rounded-md border bg-muted/40 p-2 text-xs text-muted-foreground">
            <span className="font-medium text-foreground">{t("settings.personalAccessTokens.summaryLabel")}</span>
            <span>{t("settings.personalAccessTokens.summaryScopes")}: {scopeSummary}</span>
            <span>{groupSummary}</span>
            <span>{t("settings.personalAccessTokens.summaryEffective")}</span>
          </div>

          {error ? (
            <p role="alert" className="text-sm text-destructive">{error}</p>
          ) : null}
        </div>

        <DialogFooter>
          <Button type="button" variant="outline" disabled={busy} onClick={() => onOpenChange(false)}>
            {t("common.cancel")}
          </Button>
          <Button
            type="button"
            data-testid="personal-access-token-submit"
            disabled={!canSubmit}
            aria-busy={busy}
            onClick={submit}
          >
            {t(mode === "create"
              ? "settings.personalAccessTokens.createAction"
              : "settings.personalAccessTokens.saveAction")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
