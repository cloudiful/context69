import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";

import { ConfirmDialog } from "@/components/common/confirm-dialog";
import { Button } from "@/components/ui/button";
import { apiClient, type GitProviderConnection } from "@/services/api";
import type {
  GitProviderConnectionRequest,
  GitRepositoryRegistrationRequest,
  GitRepositorySettingsUpdateRequest,
  GitRepositorySource,
  GitWebhookOwnership,
} from "@/services/api";

import { useGroupWorkspaceContext } from "./group-workspace";
import { GitSourceDetail } from "./group-git-source-detail";
import {
  GitConnectionDialog,
  GitSourceDialog,
  GitWebhookDialog,
} from "./group-git-source-dialogs";
import { GitConnectionsTable, GitSourcesTable } from "./group-git-sources-table";

function settingsFrom(
  source: GitRepositorySource,
  overrides: Partial<GitRepositorySettingsUpdateRequest> = {},
): GitRepositorySettingsUpdateRequest {
  return {
    include_patterns: source.include_patterns,
    exclude_patterns: source.exclude_patterns,
    interval_seconds: source.interval_seconds,
    webhook_enabled: source.webhook_enabled,
    enabled: source.enabled,
    retention_count: source.retention_count,
    ...overrides,
  };
}

export function GroupGitSourcesPage() {
  const { t } = useTranslation();
  const state = useGroupWorkspaceContext();
  const queryClient = useQueryClient();
  const groupPath = state.groupPath;
  const [selectedKey, setSelectedKey] = useState<string | null>(null);
  const [sourceDialog, setSourceDialog] = useState<{ open: boolean; source: GitRepositorySource | null }>({
    open: false,
    source: null,
  });
  const [connectionDialogOpen, setConnectionDialogOpen] = useState(false);
  const [webhookSource, setWebhookSource] = useState<GitRepositorySource | null>(null);
  const [connectionToDisable, setConnectionToDisable] = useState<GitProviderConnection | null>(null);
  const [sourceToDelete, setSourceToDelete] = useState<GitRepositorySource | null>(null);
  const [busySourceKey, setBusySourceKey] = useState<string | null>(null);
  const [busyConnectionKey, setBusyConnectionKey] = useState<string | null>(null);

  useEffect(() => {
    setSelectedKey(null);
    setSourceDialog({ open: false, source: null });
    setConnectionDialogOpen(false);
    setWebhookSource(null);
    setConnectionToDisable(null);
    setSourceToDelete(null);
  }, [groupPath]);

  const sourcesQuery = useQuery({
    queryKey: ["git-sources", groupPath],
    queryFn: ({ signal }) => apiClient.listGitRepositories(groupPath, { signal }),
    retry: false,
    refetchInterval: (query) => {
      const sources = query.state.data;
      const active = sources?.some((source) => (
        source.index_status === "indexing"
        || source.index_status === "pending"
        || source.refresh_pending
        || source.deletion_pending
      ));
      return active ? 3000 : false;
    },
  });

  const connectionsQuery = useQuery({
    queryKey: ["git-connections", groupPath],
    queryFn: ({ signal }) => apiClient.listGitConnections(groupPath, { signal }),
    retry: false,
  });

  const sources = sourcesQuery.data ?? [];
  const connections = connectionsQuery.data ?? [];
  const selectedSource = sources.find((source) => source.repository_key === selectedKey) ?? null;

  function invalidateSources() {
    return Promise.all([
      queryClient.invalidateQueries({ queryKey: ["git-sources", groupPath] }),
      queryClient.invalidateQueries({ queryKey: ["git-files", groupPath] }),
      queryClient.invalidateQueries({ queryKey: ["git-file", groupPath] }),
      queryClient.invalidateQueries({ queryKey: ["git-content", groupPath] }),
      queryClient.invalidateQueries({ queryKey: ["git-code-search", groupPath] }),
    ]);
  }

  function invalidateConnections() {
    return queryClient.invalidateQueries({ queryKey: ["git-connections", groupPath] });
  }

  const createSourceMutation = useMutation({
    mutationFn: (body: GitRepositoryRegistrationRequest) => apiClient.registerGitRepository(groupPath, body),
    onSuccess: async () => {
      setSourceDialog({ open: false, source: null });
      await invalidateSources();
      toast.success(t("groups.gitSources.createAccepted"));
    },
    onError: (error) => {
      toast.error(t("common.error"), {
        description: error instanceof Error ? error.message : t("groups.gitSources.createFailed"),
      });
    },
    retry: false,
  });

  const updateSourceMutation = useMutation({
    mutationFn: ({ key, body }: { key: string; body: GitRepositorySettingsUpdateRequest }) => (
      apiClient.updateGitRepositorySettings(groupPath, key, body)
    ),
    onSuccess: async () => {
      setSourceDialog({ open: false, source: null });
      await invalidateSources();
    },
    onError: (error) => {
      toast.error(t("common.error"), {
        description: error instanceof Error ? error.message : t("groups.gitSources.updateFailed"),
      });
    },
    retry: false,
  });

  const indexSourceMutation = useMutation({
    mutationFn: ({ source, forceRebuild }: { source: GitRepositorySource; forceRebuild: boolean }) => (
      apiClient.indexGitRepository(groupPath, source.repository_key, { forceRebuild })
    ),
    onMutate: ({ source }) => setBusySourceKey(source.repository_key),
    onSuccess: async (_result, { forceRebuild }) => {
      await invalidateSources();
      toast.success(t(forceRebuild ? "groups.gitSources.rebuildAccepted" : "groups.gitSources.indexAccepted"));
    },
    onError: (error, { forceRebuild }) => {
      toast.error(t("common.error"), {
        description: error instanceof Error
          ? error.message
          : t(forceRebuild ? "groups.gitSources.rebuildFailed" : "groups.gitSources.indexFailed"),
      });
    },
    onSettled: () => setBusySourceKey(null),
    retry: false,
  });

  const toggleSourceMutation = useMutation({
    mutationFn: (source: GitRepositorySource) => apiClient.updateGitRepositorySettings(
      groupPath,
      source.repository_key,
      settingsFrom(source, { enabled: !source.enabled }),
    ),
    onMutate: (source) => setBusySourceKey(source.repository_key),
    onSuccess: async () => {
      await invalidateSources();
    },
    onError: (error) => {
      toast.error(t("common.error"), {
        description: error instanceof Error ? error.message : t("groups.gitSources.updateFailed"),
      });
    },
    onSettled: () => setBusySourceKey(null),
    retry: false,
  });

  const deleteSourceMutation = useMutation({
    mutationFn: (source: GitRepositorySource) => apiClient.deleteGitRepository(groupPath, source.repository_key),
    onMutate: (source) => setBusySourceKey(source.repository_key),
    onSuccess: async (_result, source) => {
      setSourceToDelete(null);
      setSelectedKey((current) => (current === source.repository_key ? null : current));
      await invalidateSources();
      toast.success(t("groups.gitSources.deleteAccepted"));
    },
    onError: (error) => {
      toast.error(t("common.error"), {
        description: error instanceof Error ? error.message : t("groups.gitSources.deleteFailed"),
      });
    },
    onSettled: () => setBusySourceKey(null),
    retry: false,
  });

  const createConnectionMutation = useMutation({
    mutationFn: ({ key, body }: { key: string; body: GitProviderConnectionRequest }) => (
      apiClient.createGitConnection(groupPath, key, body)
    ),
    onSuccess: async () => {
      setConnectionDialogOpen(false);
      await invalidateConnections();
      toast.success(t("groups.gitSources.connectionCreated"));
    },
    onError: (error) => {
      toast.error(t("common.error"), {
        description: error instanceof Error ? error.message : t("groups.gitSources.connectionCreateFailed"),
      });
    },
    retry: false,
  });

  const toggleConnectionMutation = useMutation({
    mutationFn: (connection: GitProviderConnection) => (
      connection.disabled
        ? apiClient.enableGitConnection(groupPath, connection.connection_key)
        : apiClient.disableGitConnection(groupPath, connection.connection_key)
    ),
    onMutate: (connection) => setBusyConnectionKey(connection.connection_key),
    onSuccess: async () => {
      setConnectionToDisable(null);
      await invalidateConnections();
    },
    onError: (error) => {
      toast.error(t("common.error"), {
        description: error instanceof Error ? error.message : t("groups.gitSources.connectionUpdateFailed"),
      });
    },
    onSettled: () => setBusyConnectionKey(null),
    retry: false,
  });

  const createWebhookMutation = useMutation({
    mutationFn: ({ source, body }: {
      source: GitRepositorySource;
      body: { provider: GitRepositorySource["provider"]; external_hook_id: string; ownership: GitWebhookOwnership; signing_secret?: string };
    }) => apiClient.createGitWebhookRegistration(groupPath, source.repository_key, body),
    onSuccess: async (registration) => {
      await queryClient.invalidateQueries({
        queryKey: ["git-webhook", groupPath, registration.repository_key],
      });
      toast.success(t("groups.gitSources.webhookRegistered"));
    },
    onError: (error) => {
      toast.error(t("common.error"), {
        description: error instanceof Error ? error.message : t("groups.gitSources.webhookCreateFailed"),
      });
    },
    retry: false,
  });

  const error = sourcesQuery.error ?? connectionsQuery.error;

  return (
    <div className="grid min-w-0 gap-3 overflow-y-auto" data-testid="group-git-sources">
      <header className="flex flex-wrap items-end justify-between gap-2">
        <div className="grid gap-1">
          <h2 className="text-base font-semibold">{t("groups.gitSources.title")}</h2>
          <p className="text-sm text-muted-foreground">{t("groups.gitSources.description")}</p>
        </div>
        {state.canManage ? (
          <div className="flex flex-wrap gap-2">
            <Button type="button" variant="outline" onClick={() => setConnectionDialogOpen(true)}>
              {t("groups.gitSources.addConnection")}
            </Button>
            <Button type="button" onClick={() => setSourceDialog({ open: true, source: null })}>
              {t("groups.gitSources.addSource")}
            </Button>
          </div>
        ) : null}
      </header>

      {error ? (
        <div role="alert" className="rounded-md border border-destructive/40 p-3 text-sm">
          {error instanceof Error ? error.message : t("groups.gitSources.loadFailed")}
          <Button
            type="button"
            size="sm"
            variant="outline"
            className="ml-2"
            onClick={() => {
              void sourcesQuery.refetch();
              void connectionsQuery.refetch();
            }}
          >
            {t("common.retry")}
          </Button>
        </div>
      ) : null}

      <section className="grid gap-2 rounded-lg border p-3">
        <h3 className="text-sm font-medium">{t("groups.gitSources.sourcesTitle")}</h3>
        <GitSourcesTable
          sources={sources}
          canManage={state.canManage}
          busyKey={busySourceKey}
          onSelect={(source) => setSelectedKey(source.repository_key)}
          onSync={(source) => indexSourceMutation.mutate({ source, forceRebuild: false })}
          onRebuild={(source) => indexSourceMutation.mutate({ source, forceRebuild: true })}
          onEdit={(source) => setSourceDialog({ open: true, source })}
          onToggleEnabled={(source) => toggleSourceMutation.mutate(source)}
          onWebhook={(source) => setWebhookSource(source)}
          onDelete={(source) => setSourceToDelete(source)}
        />
      </section>

      <section className="grid gap-2 rounded-lg border p-3">
        <div className="flex flex-wrap items-center justify-between gap-2">
          <h3 className="text-sm font-medium">{t("groups.gitSources.connectionsTitle")}</h3>
          <span className="text-xs text-muted-foreground">{t("groups.gitSources.connectionsDescription")}</span>
        </div>
        {connectionsQuery.error ? (
          <div role="alert" className="rounded-md border border-destructive/40 p-2 text-sm">
            {connectionsQuery.error instanceof Error ? connectionsQuery.error.message : t("groups.gitSources.connectionsLoadFailed")}
          </div>
        ) : null}
        <GitConnectionsTable
          connections={connections}
          canManage={state.canManage}
          busyKey={busyConnectionKey}
          onToggle={(connection) => {
            if (connection.disabled) {
              toggleConnectionMutation.mutate(connection);
            } else {
              setConnectionToDisable(connection);
            }
          }}
        />
      </section>

      {selectedSource ? (
        <GitSourceDetail
          key={selectedSource.repository_key}
          groupPath={groupPath}
          source={selectedSource}
          canManage={state.canManage}
          onEdit={() => setSourceDialog({ open: true, source: selectedSource })}
          onSync={() => indexSourceMutation.mutate({ source: selectedSource, forceRebuild: false })}
          onRebuild={() => indexSourceMutation.mutate({ source: selectedSource, forceRebuild: true })}
          onWebhook={() => setWebhookSource(selectedSource)}
          onDelete={() => setSourceToDelete(selectedSource)}
        />
      ) : null}

      {sourceDialog.open ? (
        <GitSourceDialog
          key={sourceDialog.source?.repository_key ?? "new"}
          open={sourceDialog.open}
          source={sourceDialog.source}
          connections={connections}
          busy={createSourceMutation.isPending || updateSourceMutation.isPending}
          onOpenChange={(open) => setSourceDialog({ open, source: open ? sourceDialog.source : null })}
          onCreate={async (body) => { await createSourceMutation.mutateAsync(body); }}
          onUpdate={async (body) => {
            if (sourceDialog.source) {
              await updateSourceMutation.mutateAsync({ key: sourceDialog.source.repository_key, body });
            }
          }}
        />
      ) : null}

      {connectionDialogOpen ? (
        <GitConnectionDialog
          open={connectionDialogOpen}
          connections={connections}
          busy={createConnectionMutation.isPending}
          onOpenChange={setConnectionDialogOpen}
          onCreate={async (key, body) => { await createConnectionMutation.mutateAsync({ key, body }); }}
        />
      ) : null}

      {webhookSource ? (
        <GitWebhookDialog
          key={webhookSource.repository_key}
          open={!!webhookSource}
          groupPath={groupPath}
          source={webhookSource}
          busy={createWebhookMutation.isPending}
          onOpenChange={(open) => { if (!open) setWebhookSource(null); }}
          onCreate={async (body) => { await createWebhookMutation.mutateAsync({ source: webhookSource, body }); }}
        />
      ) : null}

      <ConfirmDialog
        open={!!connectionToDisable}
        title={t("groups.gitSources.disableConnectionTitle")}
        message={t("groups.gitSources.disableConnectionConfirm", {
          name: connectionToDisable?.display_name ?? "",
        })}
        acceptLabel={t("groups.gitSources.disable")}
        onAccept={() => {
          if (connectionToDisable) {
            toggleConnectionMutation.mutate(connectionToDisable);
          }
        }}
        onCancel={() => setConnectionToDisable(null)}
      />

      <ConfirmDialog
        open={!!sourceToDelete}
        title={t("groups.gitSources.deleteTitle")}
        message={t("groups.gitSources.deleteConfirm", {
          name: sourceToDelete ? `${sourceToDelete.owner}/${sourceToDelete.name}` : "",
        })}
        acceptLabel={t("groups.gitSources.delete")}
        onAccept={() => {
          if (sourceToDelete) {
            deleteSourceMutation.mutate(sourceToDelete);
          }
        }}
        onCancel={() => setSourceToDelete(null)}
      />
    </div>
  );
}
