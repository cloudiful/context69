import {
  columnVisibilityFeature,
  flexRender,
  rowSelectionFeature,
  tableFeatures,
  type ReactTable,
  type RowData,
} from "@tanstack/react-table";

import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";

export const dataTableFeatures = tableFeatures({
  columnVisibilityFeature,
  rowSelectionFeature,
});

export function renderTable<TData extends RowData>(
  table: ReactTable<typeof dataTableFeatures, TData>,
  emptyLabel: string,
  rowTestId: string,
) {
  return (
    <Table>
      <TableHeader>
        {table.getHeaderGroups().map((headerGroup) => (
          <TableRow key={headerGroup.id}>
            {headerGroup.headers.map((header) => (
              <TableHead key={header.id}>
                {header.isPlaceholder
                  ? null
                  : flexRender(header.column.columnDef.header, header.getContext())}
              </TableHead>
            ))}
          </TableRow>
        ))}
      </TableHeader>
      <TableBody>
        {table.getRowModel().rows.length === 0 ? (
          <TableRow>
            <TableCell colSpan={table.getAllColumns().length} className="py-6 text-sm text-muted-foreground">
              {emptyLabel}
            </TableCell>
          </TableRow>
        ) : table.getRowModel().rows.map((row) => (
          <TableRow key={row.id} data-testid={rowTestId} data-selected={row.getIsSelected()}>
            {row.getVisibleCells().map((cell) => (
              <TableCell key={cell.id}>
                {flexRender(cell.column.columnDef.cell, cell.getContext())}
              </TableCell>
            ))}
          </TableRow>
        ))}
      </TableBody>
    </Table>
  );
}
