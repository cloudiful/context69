import { useEffect, useState } from "react";
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
import type {
  GroupMemberResponse,
  GroupResponse,
  MembershipRole,
  UserDirectoryEntryResponse,
  Visibility,
} from "@/services/api";

import { membershipRoleLabel } from "./group-labels";
import { groupPathOf } from "./permissions";
import type { GroupEntityDraft } from "./group-workspace";

const VISIBILITIES: Visibility[] = ["private", "public"];
const ROLES: MembershipRole[] = ["owner", "maintainer", "viewer"];

export function GroupEntityDialog({
  open,
  title,
  busy,
  submitLabel,
  showKey,
  initialKey,
  initialName,
  initialVisibility,
  onOpenChange,
  onSubmit,
}: {
  open: boolean;
  title: string;
  busy: boolean;
  submitLabel: string;
  showKey: boolean;
  initialKey?: string;
  initialName?: string;
  initialVisibility?: Visibility;
  onOpenChange: (open: boolean) => void;
  onSubmit: (draft: GroupEntityDraft) => void;
}) {
  const { t } = useTranslation();
  const [key, setKey] = useState("");
  const [name, setName] = useState("");
  const [visibility, setVisibility] = useState<Visibility>("private");

  useEffect(() => {
    if (!open) {
      return;
    }
    setKey(initialKey ?? "");
    setName(initialName ?? "");
    setVisibility(initialVisibility ?? "private");
  }, [initialKey, initialName, initialVisibility, open]);

  const canSubmit = name.trim().length > 0 && (!showKey || key.trim().length > 0);

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent aria-describedby={undefined}>
        <DialogHeader>
          <DialogTitle>{title}</DialogTitle>
        </DialogHeader>
        <div className="grid gap-3">
          {showKey ? (
            <FormField label={t("groups.groupKey")} htmlFor="group-entity-key">
              <Input
                id="group-entity-key"
                value={key}
                data-testid="group-entity-key"
                onChange={(event) => setKey(event.target.value)}
              />
            </FormField>
          ) : null}
          <FormField label={t("groups.groupName")} htmlFor="group-entity-name">
            <Input
              id="group-entity-name"
              value={name}
              data-testid="group-entity-name"
              onChange={(event) => setName(event.target.value)}
            />
          </FormField>
          <FormField label={t("groups.visibility")} htmlFor="group-entity-visibility">
            <Select
              value={visibility}
              onValueChange={(next) => setVisibility(next === "public" ? "public" : "private")}
            >
              <SelectTrigger id="group-entity-visibility" className="w-full">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {VISIBILITIES.map((option) => (
                  <SelectItem key={option} value={option}>
                    {t(`groups.visibilityOptions.${option}`)}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </FormField>
        </div>
        <DialogFooter>
          <Button type="button" variant="outline" onClick={() => onOpenChange(false)}>
            {t("common.cancel")}
          </Button>
          <Button
            type="button"
            disabled={busy || !canSubmit}
            aria-busy={busy}
            onClick={() => onSubmit({ key, name, visibility })}
          >
            {submitLabel}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

export function GroupMemberDialog({
  open,
  busy,
  editing,
  suggestions,
  canAssignOwner,
  onOpenChange,
  onSearch,
  onSubmit,
}: {
  open: boolean;
  busy: boolean;
  editing: GroupMemberResponse | null;
  suggestions: UserDirectoryEntryResponse[];
  canAssignOwner: boolean;
  onOpenChange: (open: boolean) => void;
  onSearch: (query: string) => void;
  onSubmit: (payload: { login_name: string; role: MembershipRole }) => void;
}) {
  const { t } = useTranslation();
  const [role, setRole] = useState<MembershipRole>("viewer");
  const [query, setQuery] = useState("");
  const [selected, setSelected] = useState<UserDirectoryEntryResponse | null>(null);

  useEffect(() => {
    if (!open) {
      return;
    }
    setRole(editing?.role ?? "viewer");
    setQuery("");
    setSelected(editing
      ? { user_id: editing.user_id, login_name: editing.login_name, display_name: editing.display_name }
      : null);
  }, [editing, open]);

  useEffect(() => {
    if (open && !editing) {
      onSearch("");
    }
  }, [editing, onSearch, open]);

  const loginName = editing?.login_name ?? selected?.login_name ?? "";
  const roles = canAssignOwner ? ROLES : ROLES.filter((value) => value !== "owner");

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>{editing ? t("members.editTitle") : t("members.addTitle")}</DialogTitle>
          <DialogDescription>{editing ? editing.login_name : t("members.searchUserPlaceholder")}</DialogDescription>
        </DialogHeader>
        <div className="grid gap-3">
          {editing ? (
            <FormField label={t("members.loginName")} htmlFor="member-login">
              <Input id="member-login" value={editing.login_name} readOnly />
            </FormField>
          ) : (
            <FormField label={t("members.user")} htmlFor="member-search">
              <Input
                id="member-search"
                value={query}
                data-testid="member-search"
                placeholder={t("members.searchUserPlaceholder")}
                onChange={(event) => {
                  setQuery(event.target.value);
                  onSearch(event.target.value);
                }}
              />
              <div className="max-h-48 overflow-y-auto rounded-md border" role="listbox" aria-label={t("members.user")}>
                {suggestions.length === 0 ? (
                  <p className="p-2 text-xs text-muted-foreground">{t("members.noCandidates")}</p>
                ) : suggestions.map((candidate) => (
                  <button
                    key={candidate.login_name}
                    type="button"
                    role="option"
                    aria-selected={selected?.login_name === candidate.login_name}
                    className="flex w-full items-center justify-between gap-2 border-b px-2 py-1.5 text-left text-sm last:border-b-0 hover:bg-accent aria-selected:bg-accent"
                    data-testid="member-candidate"
                    onClick={() => setSelected(candidate)}
                  >
                    <span>{candidate.display_name}</span>
                    <span className="text-xs text-muted-foreground">{candidate.login_name}</span>
                  </button>
                ))}
              </div>
            </FormField>
          )}
          <FormField label={t("members.role")} htmlFor="member-role">
            <Select value={role} onValueChange={(next) => setRole(next as MembershipRole)}>
              <SelectTrigger id="member-role" className="w-full">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {roles.map((option) => (
                  <SelectItem key={option} value={option}>{membershipRoleLabel(t, option)}</SelectItem>
                ))}
              </SelectContent>
            </Select>
          </FormField>
        </div>
        <DialogFooter>
          <Button type="button" variant="outline" onClick={() => onOpenChange(false)}>
            {t("common.cancel")}
          </Button>
          <Button
            type="button"
            disabled={busy || !loginName}
            aria-busy={busy}
            onClick={() => onSubmit({ login_name: loginName, role })}
          >
            {t("common.save")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

export function MoveGroupDialog({
  open,
  busy,
  moving,
  suggestions,
  onOpenChange,
  onSearch,
  onSubmit,
}: {
  open: boolean;
  busy: boolean;
  moving: GroupResponse | null;
  suggestions: GroupResponse[];
  onOpenChange: (open: boolean) => void;
  onSearch: (query: string) => void;
  onSubmit: (target: GroupResponse | null) => void;
}) {
  const { t } = useTranslation();
  const [targetPath, setTargetPath] = useState<string>("__root__");

  useEffect(() => {
    if (open) {
      setTargetPath("__root__");
    }
  }, [open]);

  const target = suggestions.find((candidate) => groupPathOf(candidate) === targetPath) ?? null;

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>{t("groups.moveGroup")}</DialogTitle>
          <DialogDescription>{moving ? groupPathOf(moving) : ""}</DialogDescription>
        </DialogHeader>
        <div className="grid gap-2">
          <FormField label={t("groups.targetGroup")} htmlFor="move-group-target">
            <Input
              id="move-group-target"
              value={targetPath === "__root__" ? "" : targetPath}
              placeholder={t("groups.selectTargetGroup")}
              data-testid="move-group-search"
              onChange={(event) => {
                setTargetPath(event.target.value);
                onSearch(event.target.value);
              }}
            />
          </FormField>
          <div className="max-h-48 overflow-y-auto rounded-md border" role="listbox" aria-label={t("groups.targetGroup")}>
            <button
              type="button"
              role="option"
              aria-selected={targetPath === "__root__"}
              className="flex w-full items-center border-b px-2 py-1.5 text-left text-sm hover:bg-accent aria-selected:bg-accent"
              onClick={() => setTargetPath("__root__")}
            >
              {t("groups.noParentGroup")}
            </button>
            {suggestions.map((candidate) => {
              const path = groupPathOf(candidate);
              return (
                <button
                  key={candidate.group_id}
                  type="button"
                  role="option"
                  aria-selected={targetPath === path}
                  className="flex w-full flex-col items-start border-b px-2 py-1.5 text-left text-sm last:border-b-0 hover:bg-accent aria-selected:bg-accent"
                  data-testid="move-group-candidate"
                  onClick={() => setTargetPath(path)}
                >
                  <span>{candidate.name}</span>
                  <span className="text-xs text-muted-foreground">{path}</span>
                </button>
              );
            })}
          </div>
          <Badge variant="secondary" className="w-fit">{target ? target.name : t("groups.noParentGroup")}</Badge>
        </div>
        <DialogFooter>
          <Button type="button" variant="outline" onClick={() => onOpenChange(false)}>
            {t("common.cancel")}
          </Button>
          <Button
            type="button"
            disabled={busy}
            aria-busy={busy}
            onClick={() => onSubmit(targetPath === "__root__" ? null : target)}
          >
            {t("common.move")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
