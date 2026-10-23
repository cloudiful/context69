import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";

import { TablePagination } from "@/components/common/table-pagination";
import { FormField } from "@/components/common/form-field";
import { Badge } from "@/components/ui/badge";
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
import { Input } from "@/components/ui/input";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { apiClient } from "@/services/api";
import type { CreateMetadataIndexRequest, MetadataIndexResponse } from "@/services/api";

type MetadataDataType = CreateMetadataIndexRequest["data_type"];
type MetadataValueKind = CreateMetadataIndexRequest["value_kind"];

const DATA_TYPES: MetadataDataType[] = ["keyword", "integer", "float", "boolean", "datetime"];
const VALUE_KINDS: MetadataValueKind[] = ["scalar", "array"];
const PAGE_SIZE = 50;
const MAX_POLLS = 20;

function statusVariant(status: MetadataIndexResponse["status"]): "default" | "secondary" | "destructive" | "outline" {
  if (status === "ready") {
    return "default";
  }
  if (status === "failed") {
    return "destructive";
  }
  return "secondary";
}

export function MetadataIndexesSection({
  groupPath,
  canManage,
}: {
  groupPath: string;
  canManage: boolean;
}) {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const [sourceKey, setSourceKey] = useState("");
  const [activeSourceKey, setActiveSourceKey] = useState("");
  const [page, setPage] = useState(1);
  const [pageSize, setPageSize] = useState(PAGE_SIZE);
  const [dialogOpen, setDialogOpen] = useState(false);
  const [path, setPath] = useState("");
  const [dataType, setDataType] = useState<MetadataDataType>("keyword");
  const [valueKind, setValueKind] = useState<MetadataValueKind>("scalar");
  const [sortable, setSortable] = useState(false);

  useEffect(() => {
    setSourceKey("");
    setActiveSourceKey("");
    setPage(1);
  }, [groupPath]);

  const indexesQuery = useQuery({
    queryKey: ["metadata-indexes", groupPath, activeSourceKey, page, pageSize],
    queryFn: ({ signal }) => apiClient.listMetadataIndexes(groupPath, activeSourceKey, {
      page,
      pageSize,
    }, { signal }),
    enabled: !!activeSourceKey,
    retry: false,
    refetchInterval: (query) => {
      const data = query.state.data;
      const unsettled = data?.items.some((row) => row.status === "building" || row.status === "deleting");
      return unsettled && query.state.dataUpdateCount <= MAX_POLLS ? 2000 : false;
    },
  });

  const rows = indexesQuery.data?.items ?? [];
  const pagination = indexesQuery.data?.pagination ?? null;

  function invalidate() {
    return queryClient.invalidateQueries({ queryKey: ["metadata-indexes", groupPath] });
  }

  const createMutation = useMutation({
    mutationFn: () => apiClient.createMetadataIndex(groupPath, activeSourceKey, {
      path: path.trim(),
      data_type: dataType,
      value_kind: valueKind,
      sortable,
    }),
    onSuccess: async () => {
      setDialogOpen(false);
      setPath("");
      await invalidate();
    },
    onError: (error) => {
      toast.error(t("common.error"), {
        description: error instanceof Error ? error.message : t("metadataIndexes.loadFailed"),
      });
    },
    retry: false,
  });

  const retryMutation = useMutation({
    mutationFn: (row: MetadataIndexResponse) => apiClient.retryMetadataIndex(groupPath, row.index_id),
    onSuccess: () => invalidate(),
    onError: (error) => {
      toast.error(t("common.error"), {
        description: error instanceof Error ? error.message : t("metadataIndexes.loadFailed"),
      });
    },
    retry: false,
  });

  const deleteMutation = useMutation({
    mutationFn: (row: MetadataIndexResponse) => apiClient.deleteMetadataIndex(groupPath, row.index_id),
    onSuccess: () => invalidate(),
    onError: (error) => {
      toast.error(t("common.error"), {
        description: error instanceof Error ? error.message : t("metadataIndexes.loadFailed"),
      });
    },
    retry: false,
  });

  const canSubmit = activeSourceKey.length > 0 && path.trim().length > 0
    && !(valueKind === "array" && sortable);

  return (
    <section className="grid gap-3 rounded-lg border p-3" data-testid="metadata-indexes">
      <div className="flex flex-wrap items-end gap-2">
        <FormField label={t("metadataIndexes.sourceKey")} htmlFor="metadata-source-key" className="min-w-56 flex-1">
          <Input
            id="metadata-source-key"
            value={sourceKey}
            data-testid="metadata-source-key"
            onChange={(event) => setSourceKey(event.target.value)}
            onKeyDown={(event) => {
              if (event.key === "Enter") {
                setActiveSourceKey(sourceKey.trim());
                setPage(1);
              }
            }}
          />
        </FormField>
        <Button
          type="button"
          variant="outline"
          disabled={indexesQuery.isFetching || !sourceKey.trim()}
          onClick={() => {
            setActiveSourceKey(sourceKey.trim());
            setPage(1);
          }}
        >
          {t("common.refresh")}
        </Button>
        {canManage ? (
          <Button
            type="button"
            disabled={!activeSourceKey}
            data-testid="metadata-add"
            onClick={() => setDialogOpen(true)}
          >
            {t("metadataIndexes.add")}
          </Button>
        ) : null}
      </div>

      {indexesQuery.error ? (
        <p role="alert" className="text-sm text-destructive">
          {indexesQuery.error instanceof Error ? indexesQuery.error.message : t("metadataIndexes.loadFailed")}
        </p>
      ) : null}

      <div className="min-w-0 overflow-x-auto">
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead>{t("metadataIndexes.path")}</TableHead>
              <TableHead>{t("metadataIndexes.type")}</TableHead>
              <TableHead>{t("metadataIndexes.kind")}</TableHead>
              <TableHead>{t("metadataIndexes.status")}</TableHead>
              {canManage ? <TableHead>{t("common.actions")}</TableHead> : null}
            </TableRow>
          </TableHeader>
          <TableBody>
            {rows.length === 0 ? (
              <TableRow>
                <TableCell colSpan={canManage ? 5 : 4} className="py-6 text-sm text-muted-foreground">
                  {t("metadataIndexes.empty")}
                </TableCell>
              </TableRow>
            ) : rows.map((row) => (
              <TableRow key={row.index_id} data-testid="metadata-row">
                <TableCell className="font-mono text-xs">{row.path}</TableCell>
                <TableCell className="text-sm">{row.data_type}</TableCell>
                <TableCell className="text-sm">{row.value_kind}</TableCell>
                <TableCell>
                  <Badge variant={statusVariant(row.status)} title={row.error_message ?? undefined}>
                    {row.status}
                  </Badge>
                </TableCell>
                {canManage ? (
                  <TableCell>
                    <div className="flex gap-1">
                      {row.status === "failed" ? (
                        <Button
                          type="button"
                          size="sm"
                          variant="outline"
                          disabled={retryMutation.isPending}
                          onClick={() => retryMutation.mutate(row)}
                        >
                          {t("common.retry")}
                        </Button>
                      ) : null}
                      <Button
                        type="button"
                        size="sm"
                        variant="ghost"
                        disabled={deleteMutation.isPending}
                        onClick={() => deleteMutation.mutate(row)}
                      >
                        {t("common.delete")}
                      </Button>
                    </div>
                  </TableCell>
                ) : null}
              </TableRow>
            ))}
          </TableBody>
        </Table>
      </div>

      {pagination ? (
        <TablePagination
          pagination={pagination}
          onPage={setPage}
          onPageSize={(size) => {
            setPageSize(size);
            setPage(1);
          }}
        />
      ) : null}

      <Dialog open={dialogOpen} onOpenChange={setDialogOpen}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>{t("metadataIndexes.add")}</DialogTitle>
            <DialogDescription>{t("metadataIndexes.addDescription", { source: activeSourceKey })}</DialogDescription>
          </DialogHeader>
          <div className="grid gap-3">
            <FormField label={t("metadataIndexes.path")} htmlFor="metadata-path">
              <Input
                id="metadata-path"
                value={path}
                autoFocus
                data-testid="metadata-path"
                onChange={(event) => setPath(event.target.value)}
              />
            </FormField>
            <FormField label={t("metadataIndexes.type")} htmlFor="metadata-type">
              <Select value={dataType} onValueChange={(next) => setDataType(next as MetadataDataType)}>
                <SelectTrigger id="metadata-type" className="w-full"><SelectValue /></SelectTrigger>
                <SelectContent>
                  {DATA_TYPES.map((option) => <SelectItem key={option} value={option}>{option}</SelectItem>)}
                </SelectContent>
              </Select>
            </FormField>
            <FormField label={t("metadataIndexes.kind")} htmlFor="metadata-kind">
              <Select value={valueKind} onValueChange={(next) => setValueKind(next as MetadataValueKind)}>
                <SelectTrigger id="metadata-kind" className="w-full"><SelectValue /></SelectTrigger>
                <SelectContent>
                  {VALUE_KINDS.map((option) => <SelectItem key={option} value={option}>{option}</SelectItem>)}
                </SelectContent>
              </Select>
            </FormField>
            <label className="flex items-center gap-2 text-sm">
              <Checkbox
                checked={sortable}
                disabled={valueKind === "array"}
                onCheckedChange={(checked) => setSortable(checked === true)}
              />
              {t("metadataIndexes.sortable")}
            </label>
          </div>
          <DialogFooter>
            <Button type="button" variant="outline" onClick={() => setDialogOpen(false)}>
              {t("common.cancel")}
            </Button>
            <Button
              type="button"
              disabled={!canSubmit || createMutation.isPending}
              aria-busy={createMutation.isPending}
              onClick={() => createMutation.mutate()}
            >
              {t("common.create")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </section>
  );
}
