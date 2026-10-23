import { ArrowDown, ArrowUp, Search } from "lucide-react";
import { useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";

import { ConfirmDialog } from "@/components/common/confirm-dialog";
import { SettingsBlock, SettingsSection } from "@/components/common/settings-section";
import { TablePagination } from "@/components/common/table-pagination";
import { TextField } from "@/components/common/text-field";
import { ToggleField } from "@/components/common/toggle-field";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { InputGroup, InputGroupAddon, InputGroupInput } from "@/components/ui/input-group";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";
import { SettingsGate } from "@/features/settings/settings-gate";
import { useSettings } from "@/features/settings/settings-context";
import { formatTimestamp } from "@/lib/format";
import type { AdminUserResponse, AdminUserSortBy } from "@/services/api";

type SortField = Extract<AdminUserSortBy, "login_name" | "display_name" | "created_at">;

export function AdminUsersSection() {
  const { t } = useTranslation();
  const { adminUsers } = useSettings();

  const [createOpen, setCreateOpen] = useState(false);
  const [createForm, setCreateForm] = useState({ login_name: "", display_name: "", password: "", is_admin: false });
  const [editing, setEditing] = useState<AdminUserResponse | null>(null);
  const [editForm, setEditForm] = useState({ display_name: "", is_admin: false });
  const [resetting, setResetting] = useState<AdminUserResponse | null>(null);
  const [resetPassword, setResetPassword] = useState("");
  const [statusTarget, setStatusTarget] = useState<{ user: AdminUserResponse; enable: boolean } | null>(null);

  function toggleSort(field: SortField) {
    const current = adminUsers.sort;
    if (current?.field !== field) {
      adminUsers.changeSort({ field, direction: "asc" });
      return;
    }
    adminUsers.changeSort({ field, direction: current.direction === "asc" ? "desc" : "asc" });
  }

  function sortIcon(field: SortField) {
    if (adminUsers.sort?.field !== field) {
      return null;
    }
    return adminUsers.sort.direction === "asc"
      ? <ArrowUp aria-hidden="true" className="size-3" />
      : <ArrowDown aria-hidden="true" className="size-3" />;
  }

  return (
    <SettingsSection title={t("adminUsers.title")}>
      <SettingsGate>
      <SettingsBlock id="settings-admin-users" compact>
        <div className="flex flex-wrap items-center justify-between gap-2">
          <InputGroup className="w-56">
            <InputGroupAddon>
              <Search aria-hidden="true" />
            </InputGroupAddon>
            <InputGroupInput
              value={adminUsers.query}
              placeholder={t("adminUsers.loginName")}
              aria-label={t("adminUsers.loginName")}
              onChange={(event) => adminUsers.setQuery(event.target.value)}
            />
          </InputGroup>
          <Button type="button" size="sm" disabled={adminUsers.createBusy} onClick={() => setCreateOpen(true)}>
            {t("adminUsers.create")}
          </Button>
        </div>

        <Table>
          <TableHeader>
            <TableRow>
              <SortableHead label={t("adminUsers.loginName")} onClick={() => toggleSort("login_name")} icon={sortIcon("login_name")} />
              <SortableHead label={t("adminUsers.displayName")} onClick={() => toggleSort("display_name")} icon={sortIcon("display_name")} />
              <TableHead>{t("adminUsers.isAdmin")}</TableHead>
              <TableHead>{t("adminUsers.status")}</TableHead>
              <SortableHead label={t("adminUsers.createdAt")} onClick={() => toggleSort("created_at")} icon={sortIcon("created_at")} />
              <TableHead>{t("common.actions")}</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {adminUsers.users.length === 0 ? (
              <TableRow>
                <TableCell colSpan={6} className="text-sm text-muted-foreground">
                  {adminUsers.busy ? t("common.loading") : t("adminUsers.loadFailed")}
                </TableCell>
              </TableRow>
            ) : (
              adminUsers.users.map((user) => (
                <TableRow key={user.login_name}>
                  <TableCell className="font-medium">{user.login_name}</TableCell>
                  <TableCell>{user.display_name}</TableCell>
                  <TableCell>
                    <Badge variant={user.is_admin ? "outline" : "secondary"}>
                      {user.is_admin ? t("common.yes") : t("common.no")}
                    </Badge>
                  </TableCell>
                  <TableCell>
                    <Badge variant={user.disabled_at ? "secondary" : "outline"} className="whitespace-nowrap">
                      {user.disabled_at ? t("adminUsers.disabled") : t("adminUsers.active")}
                    </Badge>
                  </TableCell>
                  <TableCell className="whitespace-nowrap">{formatTimestamp(user.created_at)}</TableCell>
                  <TableCell>
                    <div className="flex flex-nowrap items-center gap-1">
                      <Button
                        type="button"
                        variant="outline"
                        size="sm"
                        onClick={() => {
                          setEditing(user);
                          setEditForm({ display_name: user.display_name, is_admin: user.is_admin });
                        }}
                      >
                        {t("common.edit")}
                      </Button>
                      <Button
                        type="button"
                        variant="outline"
                        size="sm"
                        onClick={() => {
                          setResetting(user);
                          setResetPassword("");
                        }}
                      >
                        {t("adminUsers.resetPasswordAction")}
                      </Button>
                      <Button
                        type="button"
                        variant={user.disabled_at ? "outline" : "destructive"}
                        size="sm"
                        onClick={() => setStatusTarget({ user, enable: !!user.disabled_at })}
                      >
                        {user.disabled_at ? t("adminUsers.enableUser") : t("adminUsers.disableUser")}
                      </Button>
                    </div>
                  </TableCell>
                </TableRow>
              ))
            )}
          </TableBody>
        </Table>
        <TablePagination
          pagination={adminUsers.pagination}
          onPage={adminUsers.changePage}
          onPageSize={adminUsers.changePageSize}
        />
      </SettingsBlock>
      </SettingsGate>

      <Dialog open={createOpen} onOpenChange={setCreateOpen}>
        <DialogContent className="w-[30rem] max-w-[96vw]">
          <DialogHeader>
            <DialogTitle>{t("adminUsers.create")}</DialogTitle>
          </DialogHeader>
          <div className="grid gap-3">
            <TextField
              id="admin-user-create-login-name"
              label={t("adminUsers.loginName")}
              value={createForm.login_name}
              onChange={(value) => setCreateForm((form) => ({ ...form, login_name: value }))}
            />
            <TextField
              id="admin-user-create-display-name"
              label={t("adminUsers.displayName")}
              value={createForm.display_name}
              onChange={(value) => setCreateForm((form) => ({ ...form, display_name: value }))}
            />
            <TextField
              id="admin-user-create-password"
              type="password"
              autoComplete="new-password"
              label={t("adminUsers.password")}
              value={createForm.password}
              onChange={(value) => setCreateForm((form) => ({ ...form, password: value }))}
            />
            <ToggleField
              id="admin-user-create-is-admin"
              label={t("adminUsers.isAdmin")}
              checked={createForm.is_admin}
              onCheckedChange={(checked) => setCreateForm((form) => ({ ...form, is_admin: checked }))}
            />
          </div>
          <DialogFooter>
            <Button type="button" variant="outline" onClick={() => setCreateOpen(false)}>
              {t("common.cancel")}
            </Button>
            <Button
              type="button"
              disabled={adminUsers.createBusy
                || !createForm.login_name.trim()
                || !createForm.display_name.trim()
                || !createForm.password.trim()}
              onClick={() => {
                adminUsers.createUser({
                  login_name: createForm.login_name.trim(),
                  display_name: createForm.display_name.trim(),
                  password: createForm.password,
                  is_admin: createForm.is_admin,
                });
                setCreateForm({ login_name: "", display_name: "", password: "", is_admin: false });
                setCreateOpen(false);
              }}
            >
              {t("adminUsers.create")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <Dialog open={editing !== null} onOpenChange={(open) => {
        if (!open) {
          setEditing(null);
        }
      }}>
        <DialogContent className="w-[28rem] max-w-[96vw]">
          <DialogHeader>
            <DialogTitle>{t("common.edit")}</DialogTitle>
          </DialogHeader>
          <div className="grid gap-3">
            <TextField
              id="admin-user-edit-display-name"
              label={t("adminUsers.displayName")}
              value={editForm.display_name}
              onChange={(value) => setEditForm((form) => ({ ...form, display_name: value }))}
            />
            <ToggleField
              id="admin-user-edit-is-admin"
              label={t("adminUsers.isAdmin")}
              checked={editForm.is_admin}
              onCheckedChange={(checked) => setEditForm((form) => ({ ...form, is_admin: checked }))}
            />
          </div>
          <DialogFooter>
            <Button type="button" variant="outline" onClick={() => setEditing(null)}>
              {t("common.cancel")}
            </Button>
            <Button
              type="button"
              disabled={adminUsers.busy || !editForm.display_name.trim()}
              onClick={() => {
                if (editing) {
                  adminUsers.updateUser({
                    login_name: editing.login_name,
                    display_name: editForm.display_name.trim(),
                    is_admin: editForm.is_admin,
                  });
                }
                setEditing(null);
              }}
            >
              {t("common.save")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <Dialog open={resetting !== null} onOpenChange={(open) => {
        if (!open) {
          setResetting(null);
        }
      }}>
        <DialogContent className="w-[28rem] max-w-[96vw]">
          <DialogHeader>
            <DialogTitle>{t("adminUsers.resetPasswordAction")}</DialogTitle>
          </DialogHeader>
          <TextField
            id="admin-user-reset-password"
            type="password"
            autoComplete="new-password"
            label={t("adminUsers.resetPassword")}
            value={resetPassword}
            onChange={setResetPassword}
          />
          <DialogFooter>
            <Button type="button" variant="outline" onClick={() => setResetting(null)}>
              {t("common.cancel")}
            </Button>
            <Button
              type="button"
              disabled={adminUsers.busy || !resetPassword.trim()}
              onClick={() => {
                if (resetting) {
                  adminUsers.resetPassword({ login_name: resetting.login_name, password: resetPassword });
                }
                setResetting(null);
              }}
            >
              {t("adminUsers.resetPasswordAction")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <ConfirmDialog
        open={statusTarget !== null}
        title={statusTarget?.enable ? t("adminUsers.enableUser") : t("adminUsers.disableUser")}
        message={statusTarget?.enable
          ? t("adminUsers.enableConfirm", { loginName: statusTarget.user.login_name })
          : statusTarget
            ? t("adminUsers.disableConfirm", { loginName: statusTarget.user.login_name })
            : ""}
        acceptLabel={statusTarget?.enable ? t("adminUsers.enableUser") : t("adminUsers.disableUser")}
        onAccept={() => {
          const target = statusTarget;
          setStatusTarget(null);
          if (target) {
            if (target.enable) {
              adminUsers.enableUser(target.user.login_name);
            } else {
              adminUsers.disableUser(target.user.login_name);
            }
          }
        }}
        onCancel={() => setStatusTarget(null)}
      />
    </SettingsSection>
  );
}

function SortableHead({
  label,
  onClick,
  icon,
}: {
  label: string;
  onClick: () => void;
  icon: ReactNode;
}) {
  return (
    <TableHead>
      <button type="button" className="inline-flex items-center gap-1" onClick={onClick}>
        {label}
        {icon}
      </button>
    </TableHead>
  );
}
