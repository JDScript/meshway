import type { ReactNode } from "react"

import { cn } from "@/lib/utils"

export type Column = { label: ReactNode; numeric?: boolean }

export type Row = {
  key: string | number
  cells: readonly ReactNode[]
  tone?: "destructive" | "warning"
}

/** The panel's tables: monospaced, one line a row, wider than the panel scrolls sideways. */
export function DataTable({
  columns,
  rows,
}: {
  columns: readonly Column[]
  rows: readonly Row[]
}) {
  const align = (index: number) =>
    columns[index]?.numeric ? "text-right tabular-nums" : "text-left"
  return (
    <div className="overflow-x-auto">
      <table className="font-mono text-meta">
        <thead className="text-muted-foreground">
          <tr>
            {columns.map((column, index) => (
              <th
                key={index}
                className={cn(
                  "pr-4 pb-1 font-normal whitespace-nowrap",
                  align(index)
                )}
              >
                {column.label}
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {rows.map((row) => (
            <tr
              key={row.key}
              className={cn(
                row.tone === "destructive" && "text-destructive",
                row.tone === "warning" && "text-warning"
              )}
            >
              {row.cells.map((cell, index) => (
                <td
                  key={index}
                  className={cn("py-0.5 pr-4 whitespace-nowrap", align(index))}
                >
                  {cell}
                </td>
              ))}
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  )
}
