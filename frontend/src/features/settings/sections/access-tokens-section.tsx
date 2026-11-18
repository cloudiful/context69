import { Info, Plus } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";

import { ConfirmDialog } from "@/components/common/confirm-dialog";
import { FormField } from "@/components/common/form-field";
import { TablePagination } from "@/components/common/table-pagination";
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
import { SettingsBlock, SettingsSection } from "@/components/common/settings-section";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";
import { Textarea } from "@/components/ui/textarea";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { SettingsGate } from "@/features/settings/settings-gate";
import { useSettings } from "@/features/settings/settings-context";
import {
  DEFAULT_EXPIRY_DAYS,
  PERSONAL_ACCESS_TOKEN_EXPIRY_DAYS,
} from "@/features/settings/use-personal-access-tokens";
import { useAuthSession } from "@/hooks/use-auth";
import { formatDate, formatTimestamp } from "@/lib/format";
import { UTC_TIME_ZONE } from "@/lib/time-zone";
import type { PersonalAccessTokenResponse } from "@/services/api";

import { PersonalAccessTokenDialog, type PersonalAccessTokenFormValue } from "./personal-access-token-dialog";
import { resolveGroupLabels, useGroupLabelCache } from "./personal-access-token-group-labels";

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
  const { user } = useAuthSession();
  const userKey = user ? String(user.user_id) : "anonymous";
  const groupLabels = useGroupLabelCache(userKey);
  const [editor, setEditor] = useState<{ mode: "create" } | { mode: "edit"; token: PersonalAccessTokenResponse } | null>(null);
  const [rotateTarget, setRotateTarget] = useState<PersonalAccessTokenResponse | null>(null);
  const [revokeTarget, setRevokeTarget] = useState<PersonalAccessTokenResponse | null>(null);

  const restrictedGroupIds = useMemo(() => {
    const ids = new Set<number>();
    for (const token of tokens.items) {
      if (token.group_scope.mode === "groups") {
        for (const id of token.group_scope.group_ids) {
          ids.add(id);
        }
      }
    }
    return [...ids].sort((left, right) => left - right);
  }, [tokens.items]);
  const restrictedGroupKey = restrictedGroupIds.join(",");

  useEffect(() => {
    if (restrictedGroupIds.length === 0) {
      return;
    }
    let cancelled = false;
    const handle = setTimeout(() => {
      void resolveGroupLabels(userKey, restrictedGroupIds, () => cancelled);
    }, 0);
    return () => {
      cancelled = true;
      clearTimeout(handle);
    };
  }, [restrictedGroupKey, userKey]);

  function groupRangeSummary(ids: number[]): string {
    const names = ids.map((id) => (
      groupLabels.get(id)?.name ?? t("settings.personalAccessTokens.groupFallback", { id })
    ));
    if (names.length <= 2) {
      return names.join(", ");
    }
    return `${names.slice(0, 2).join(", ")} +${names.length - 2}`;
  }

  function openCreate() {
    tokens.clearActionError();
    setEditor({ mode: "create" });
  }

  function openEdit(token: PersonalAccessTokenResponse) {
    tokens.clearActionError();
    setEditor({ mode: "edit", token });
  }

  function openRotate(token: PersonalAccessTokenResponse) {
    tokens.clearActionError();
    setRotateTarget(token);
  }

  function closeEditor() {
    if (tokens.busyAction === "create" || tokens.busyAction === "update") {
      return;
    }
    setEditor(null);
    tokens.clearActionError();
  }

  function closeRotate() {
    if (tokens.busyAction === "rotate") {
      return;
    }
    setRotateTarget(null);
    tokens.clearActionError();
  }

  async function submit(value: PersonalAccessTokenFormValue) {
    const editing = editor?.mode === "edit" ? editor.token : null;
    const ok = editing
      ? await tokens.update(editing.token_id, {
        name: value.name,
        scopes: value.scopes,
        group_scope: value.groupScope,
      })
      : await tokens.create({
        name: value.name,
        scopes: value.scopes,
        group_scope: value.groupScope,
        expires_in_days: value.expiresInDays,
      });
    if (ok) {
      setEditor(null);
      tokens.clearActionError();
    }
  }

  const dialogBusy = tokens.busyAction === "create" || tokens.busyAction === "update";

  return (
    <SettingsSection
      title={t("settings.personalAccessTokens.title")}
      actions={(
        <>
          <Tooltip>
            <TooltipTrigger asChild>
              <Button
                type="button"
                variant="ghost"
                size="icon-sm"
                aria-label={t("settings.personalAccessTokens.usageHintLabel")}
              >
                <Info aria-hidden="true" />
              </Button>
            </TooltipTrigger>
            <TooltipContent className="max-w-xs text-xs leading-5">
              {t("settings.personalAccessTokens.usageHint")}
            </TooltipContent>
          </Tooltip>
          <Button type="button" size="sm" data-testid="personal-access-token-add" onClick={openCreate}>
            <Plus aria-hidden="true" />
            {t("settings.personalAccessTokens.addAction")}
          </Button>
        </>
      )}
    >
      <SettingsGate>
        <div className="grid gap-3">
          <SettingsBlock compact>
            <div className="overflow-x-auto">
              <Table>
                <TableHeader>
                  <TableRow>
                    <TableHead>{t("settings.personalAccessTokens.name")}</TableHead>
                    <TableHead>{t("settings.personalAccessTokens.scopes")}</TableHead>
                    <TableHead>{t("settings.personalAccessTokens.groupRange")}</TableHead>
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
                      <TableCell colSpan={8} className="text-sm text-muted-foreground">
                        {tokens.loading
                          ? t("common.loading")
                          : t("settings.personalAccessTokens.empty")}
                      </TableCell>
                    </TableRow>
                  ) : (
                    tokens.items.map((token) => {
                      const status = statusOf(token);
                      const groupIds = token.group_scope.mode === "groups" ? token.group_scope.group_ids : [];
                      return (
                        <TableRow key={token.token_id}>
                          <TableCell>
                            <div className="grid gap-0.5">
                              <span className="font-medium">{token.name}</span>
                              <span className="font-mono text-xs text-muted-foreground">{token.display_prefix}…</span>
                            </div>
                          </TableCell>
                          <TableCell>
                            <div className="flex max-w-48 flex-wrap gap-1">
                              {token.scopes.map((scope) => (
                                <Badge key={scope} variant="outline">
                                  {t(`settings.personalAccessTokens.scopeLabel.${scope}`)}
                                </Badge>
                              ))}
                            </div>
                          </TableCell>
                          <TableCell>
                            {token.group_scope.mode === "all" ? (
                              <span className="text-sm">{t("settings.personalAccessTokens.groupRangeAll")}</span>
                            ) : (
                              <Tooltip>
                                <TooltipTrigger asChild>
                                  <button
                                    type="button"
                                    className="max-w-56 rounded text-left text-sm underline decoration-dotted underline-offset-2 focus-visible:ring-2 focus-visible:ring-ring"
                                  >
                                    <span className="line-clamp-2">{groupRangeSummary(groupIds)}</span>
                                  </button>
                                </TooltipTrigger>
                                <TooltipContent>
                                  <div className="grid gap-0.5">
                                    <span className="font-medium">
                                      {t("settings.personalAccessTokens.groupRangeCount", { count: groupIds.length })}
                                    </span>
                                    {groupIds.map((id) => {
                                      const label = groupLabels.get(id);
                                      return (
                                        <span key={id}>
                                          {label
                                            ? `${label.name} — ${label.path}`
                                            : t("settings.personalAccessTokens.groupFallback", { id })}
                                        </span>
                                      );
                                    })}
                                  </div>
                                </TooltipContent>
                              </Tooltip>
                            )}
                          </TableCell>
                          <TableCell className="whitespace-nowrap">{formatTimestamp(token.created_at)}</TableCell>
                          <TableCell className="whitespace-nowrap">{formatTimestamp(token.expires_at)}</TableCell>
                          <TableCell>
                            {token.last_used_at ? (
                              (() => {
                                const precise = token.last_used_precision === "exact";
                                return (
                                  <div className="flex flex-wrap items-center gap-1 whitespace-nowrap">
                                    <span>
                                      {precise
                                        ? formatTimestamp(token.last_used_at)
                                        : formatDate(token.last_used_at, UTC_TIME_ZONE)}
                                    </span>
                                    {precise ? null : (
                                      <Tooltip>
                                        <TooltipTrigger asChild>
                                          <Badge variant="outline" className="cursor-default text-[10px]">
                                            {t("settings.personalAccessTokens.lastUsedLegacyDay")}
                                          </Badge>
                                        </TooltipTrigger>
                                        <TooltipContent>
                                          {t("settings.personalAccessTokens.lastUsedLegacyHint")}
                                        </TooltipContent>
                                      </Tooltip>
                                    )}
                                  </div>
                                );
                              })()
                            ) : (
                              <span className="text-sm text-muted-foreground">
                                {t("settings.personalAccessTokens.neverUsed")}
                              </span>
                            )}
                          </TableCell>
                          <TableCell>
                            <Badge variant={status.variant} className="whitespace-nowrap">
                              {t(status.labelKey)}
                            </Badge>
                          </TableCell>
                          <TableCell>
                            <div className="flex flex-wrap gap-1">
                              <Button
                                type="button"
                                variant="outline"
                                size="xs"
                                disabled={tokens.busyAction !== null}
                                onClick={() => openEdit(token)}
                              >
                                {t("settings.personalAccessTokens.editAction")}
                              </Button>
                              <Button
                                type="button"
                                variant="outline"
                                size="xs"
                                disabled={tokens.busyAction !== null}
                                onClick={() => openRotate(token)}
                              >
                                {t("settings.personalAccessTokens.rotateAction")}
                              </Button>
                              <Button
                                type="button"
                                variant="destructive"
                                size="xs"
                                disabled={tokens.busyAction !== null || token.revoked_at != null}
                                onClick={() => setRevokeTarget(token)}
                              >
                                {t("settings.personalAccessTokens.revokeAction")}
                              </Button>
                            </div>
                          </TableCell>
                        </TableRow>
                      );
                    })
                  )}
                </TableBody>
              </Table>
            </div>
            <TablePagination
              pagination={tokens.pagination}
              onPage={tokens.changePage}
              onPageSize={tokens.changePageSize}
            />
          </SettingsBlock>
        </div>
      </SettingsGate>

      <PersonalAccessTokenDialog
        open={editor !== null}
        mode={editor?.mode === "edit" ? "edit" : "create"}
        token={editor?.mode === "edit" ? editor.token : null}
        busy={dialogBusy}
        error={dialogBusy ? null : tokens.actionError}
        onOpenChange={(open) => {
          if (!open) {
            closeEditor();
          }
        }}
        onSubmit={(value) => void submit(value)}
      />

      <RotateTokenDialog
        open={rotateTarget !== null}
        token={rotateTarget}
        busy={tokens.busyAction === "rotate"}
        error={tokens.busyAction === "rotate" ? null : tokens.actionError}
        onOpenChange={(open) => {
          if (!open) {
            closeRotate();
          }
        }}
        onConfirm={(days) => {
          const target = rotateTarget;
          if (!target) {
            return;
          }
          void (async () => {
            const ok = await tokens.rotate(target.token_id, days);
            if (ok) {
              setRotateTarget(null);
              tokens.clearActionError();
            }
          })();
        }}
      />

      <ConfirmDialog
        open={revokeTarget !== null}
        title={t("settings.personalAccessTokens.revokeAction")}
        message={t("settings.personalAccessTokens.revokeConfirm", { name: revokeTarget?.name ?? "" })}
        acceptLabel={t("settings.personalAccessTokens.revokeAction")}
        onAccept={() => {
          const target = revokeTarget;
          setRevokeTarget(null);
          if (target) {
            void tokens.revoke(target);
          }
        }}
        onCancel={() => setRevokeTarget(null)}
      />

      <Dialog open={tokens.reveal !== null} onOpenChange={(open) => {
        if (!open) {
          tokens.dismissReveal();
        }
      }}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>{t("settings.personalAccessTokens.revealTitle")}</DialogTitle>
            <DialogDescription>{t("settings.personalAccessTokens.revealWarning")}</DialogDescription>
          </DialogHeader>
          <Textarea
            data-testid="personal-access-token-secret"
            value={tokens.reveal?.access_token ?? ""}
            readOnly
            rows={3}
            aria-label={t("settings.personalAccessTokens.revealTitle")}
            className="font-mono text-xs"
          />
          <DialogFooter>
            <Button type="button" variant="outline" onClick={() => void tokens.copy()}>
              {t("settings.personalAccessTokens.copyAction")}
            </Button>
            <Button type="button" onClick={tokens.dismissReveal}>
              {t("common.close")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </SettingsSection>
  );
}

function RotateTokenDialog({
  open,
  token,
  busy,
  error,
  onOpenChange,
  onConfirm,
}: {
  open: boolean;
  token: PersonalAccessTokenResponse | null;
  busy: boolean;
  error: string | null;
  onOpenChange: (open: boolean) => void;
  onConfirm: (expiresInDays: number) => void;
}) {
  const { t } = useTranslation();
  const [expiresInDays, setExpiresInDays] = useState<number>(DEFAULT_EXPIRY_DAYS);

  useEffect(() => {
    if (open) {
      setExpiresInDays(DEFAULT_EXPIRY_DAYS);
    }
  }, [open]);

  return (
    <Dialog open={open} onOpenChange={(next) => {
      if (!next && busy) {
        return;
      }
      onOpenChange(next);
    }}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>{t("settings.personalAccessTokens.rotateTitle")}</DialogTitle>
          <DialogDescription>
            {t("settings.personalAccessTokens.rotateDescription", { name: token?.name ?? "" })}
          </DialogDescription>
        </DialogHeader>
        <div className="grid gap-2">
          <FormField
            label={t("settings.personalAccessTokens.rotateExpiry")}
            htmlFor="personal-access-token-rotate-expiry"
          >
            <Select
              value={String(expiresInDays)}
              onValueChange={(value) => setExpiresInDays(Number(value))}
            >
              <SelectTrigger
                id="personal-access-token-rotate-expiry"
                aria-label={t("settings.personalAccessTokens.rotateExpiry")}
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
          <p className="text-xs text-muted-foreground">
            {t("settings.personalAccessTokens.rotateRevalidates")}
          </p>
          {error ? <p role="alert" className="text-sm text-destructive">{error}</p> : null}
        </div>
        <DialogFooter>
          <Button type="button" variant="outline" disabled={busy} onClick={() => onOpenChange(false)}>
            {t("common.cancel")}
          </Button>
          <Button
            type="button"
            variant="destructive"
            disabled={busy}
            aria-busy={busy}
            onClick={() => onConfirm(expiresInDays)}
          >
            {t("settings.personalAccessTokens.rotateAction")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
