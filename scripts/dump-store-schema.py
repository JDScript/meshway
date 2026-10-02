#!/usr/bin/env python3
"""Export schema only from a fresh temporary Controller store; never reads live rows."""
import argparse
import csv
import hashlib
import json
import os
from pathlib import Path
import select
import sqlite3
import subprocess
import tempfile
import time


def dump(binary, output, source_ref, temporary_parent):
    binary = binary.resolve(strict=True)
    output.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="meshway-schema-audit-", dir=temporary_parent) as directory:
        data = Path(directory) / "controller"
        child = subprocess.Popen([str(binary), "serve", "--bind", "127.0.0.1:0", "--data-dir", str(data)],
                                 stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
        try:
            deadline = time.monotonic() + 30
            startup = b""
            while b"http://127.0.0.1:" not in startup:
                if child.poll() is not None:
                    raise RuntimeError("Temporary controller exited before schema initialization")
                remaining = deadline - time.monotonic()
                if remaining <= 0:
                    raise RuntimeError("Temporary controller startup deadline exceeded")
                readable, _, _ = select.select([child.stderr], [], [], remaining)
                if readable:
                    startup = (startup + os.read(child.stderr.fileno(), 8192))[-32768:]
            database = data / "controller.sqlite3"
            with sqlite3.connect(database.as_uri() + "?mode=ro", uri=True) as connection:
                connection.row_factory = sqlite3.Row
                version = connection.execute("PRAGMA user_version").fetchone()[0]
                objects = [dict(row) for row in connection.execute(
                    "SELECT type,name,tbl_name,sql FROM sqlite_schema WHERE sql IS NOT NULL AND name NOT LIKE 'sqlite_%' ORDER BY type,name")]
                tables = []
                for table in connection.execute("PRAGMA table_list"):
                    if table["schema"] != "main" or table["name"].startswith("sqlite_"):
                        continue
                    name = table["name"]
                    columns = [dict(row) for row in connection.execute("SELECT * FROM pragma_table_xinfo(?)", [name])]
                    foreign_keys = [dict(row) for row in connection.execute("SELECT * FROM pragma_foreign_key_list(?)", [name])]
                    indexes = [dict(row) for row in connection.execute("SELECT * FROM pragma_index_list(?)", [name])]
                    tables.append({"name": name, "type": table["type"], "without_rowid": bool(table["wr"]),
                                   "strict": bool(table["strict"]), "columns": columns,
                                   "foreign_keys": foreign_keys, "indexes": indexes})
                tables.sort(key=lambda item: item["name"])
            ddl = "-- Audit snapshot only; not an installation or migration script.\n"
            ddl += f"-- Source: {source_ref}; PRAGMA user_version = {version}. No row data.\n\n"
            ddl += "\n\n".join(item["sql"].rstrip(";") + ";" for item in objects) + "\n"
            (output / "schema.sql").write_text(ddl, encoding="utf-8")
            result = {"source_ref": source_ref, "user_version": version, "tables": tables, "objects": objects,
                      "ddl_sha256": hashlib.sha256(ddl.encode()).hexdigest()}
            (output / "inventory.json").write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
            with (output / "columns.csv").open("w", encoding="utf-8", newline="") as destination:
                writer = csv.writer(destination, lineterminator="\n")
                writer.writerow(["table", "column", "type", "declared_not_null", "pk_position", "default_sql", "hidden", "strict", "without_rowid"])
                for table in tables:
                    for column in table["columns"]:
                        writer.writerow([table["name"], column["name"], column["type"], column["notnull"], column["pk"],
                                         column["dflt_value"], column["hidden"], table["strict"], table["without_rowid"]])
            lines = ["# SQLite 结构完整目录", "", f"冻结源码 `{source_ref}` 的新建临时 store；schema v{version}。仅结构，没有业务行。", "",
                     "`notnull` 是 PRAGMA 原始标记；是否允许 NULL 还须结合主键、STRICT / WITHOUT ROWID、CHECK 和 trigger 判断。", "",
                     "完整定义见同目录 `schema.sql`，机器数据为 `inventory.json` / `columns.csv`。", ""]
            for table in tables:
                lines += ["## " + table["name"], "", f"STRICT={table['strict']}；WITHOUT ROWID={table['without_rowid']}。", "",
                          "| 字段 | SQL 类型 | notnull | 主键位置 | 默认 SQL |", "| --- | --- | --- | --- | --- |"]
                for column in table["columns"]:
                    default = str(column["dflt_value"]).replace("|", "\\|") if column["dflt_value"] is not None else "—"
                    lines.append(f"| `{column['name']}` | `{column['type']}` | {column['notnull']} | {column['pk']} | `{default}` |")
                lines.append("")
                for key in table["foreign_keys"]:
                    lines.append(f"- FK `{key['from']}` → `{key['table']}.{key['to']}`；delete={key['on_delete']}，update={key['on_update']}。")
                lines.append("")
            (output / "models.md").write_text("\n".join(lines).rstrip() + "\n", encoding="utf-8")
            print(json.dumps({"user_version": version, "tables": len(tables), "columns": sum(len(table["columns"]) for table in tables),
                              "schema_objects": len(objects), "ddl_sha256": result["ddl_sha256"]}, indent=2))
        finally:
            if child.poll() is None:
                child.terminate()
            try:
                child.wait(timeout=10)
            except subprocess.TimeoutExpired:
                child.kill()
                child.wait()
            child.stderr.close()


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--controller-bin", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--source-ref", required=True)
    parser.add_argument("--temporary-parent", type=Path, default=None)
    args = parser.parse_args()
    dump(args.controller_bin, args.output, args.source_ref, args.temporary_parent)
