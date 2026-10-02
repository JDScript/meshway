#!/usr/bin/env python3
"""Read-only OpenAPI / JSON Schema inventory; emits audit snapshots, not DTOs."""
import argparse
import csv
import hashlib
import json
from pathlib import Path


METHODS = {"get", "post", "put", "patch", "delete", "head", "options", "trace"}
BOUNDS = ("format", "minimum", "maximum", "exclusiveMinimum", "exclusiveMaximum",
          "minLength", "maxLength", "pattern", "minItems", "maxItems", "uniqueItems",
          "minProperties", "maxProperties", "additionalProperties", "propertyNames",
          "enum", "const", "default", "readOnly", "writeOnly", "deprecated")


def pointer_part(value):
    return value.replace("~", "~0").replace("/", "~1")


def dereference(document, ref):
    if not ref.startswith("#/"):
        return None
    value = document
    try:
        for raw in ref[2:].split("/"):
            key = raw.replace("~1", "/").replace("~0", "~")
            value = value[int(key)] if isinstance(value, list) else value[key]
        return value
    except (KeyError, IndexError, TypeError, ValueError):
        return None


def conjunction(values):
    return False if False in values else None if None in values else True


def accepts_null(schema, document, seen=frozenset()):
    """Evaluate only the JSON literal null; unresolved refs stay explicitly unknown."""
    if isinstance(schema, bool):
        return schema
    if not isinstance(schema, dict):
        return None
    results = []
    if "$ref" in schema:
        ref = schema["$ref"]
        target = dereference(document, ref)
        results.append(None if ref in seen or target is None else accepts_null(target, document, seen | {ref}))
    if "$dynamicRef" in schema or "$recursiveRef" in schema:
        results.append(None)
    if "type" in schema:
        types = schema["type"] if isinstance(schema["type"], list) else [schema["type"]]
        results.append("null" in types)
    if "enum" in schema:
        results.append(None in schema["enum"])
    if "const" in schema:
        results.append(schema["const"] is None)
    for keyword in ("allOf", "anyOf", "oneOf"):
        if keyword not in schema:
            continue
        parts = [accepts_null(part, document, seen) for part in schema[keyword]]
        if keyword == "allOf":
            value = conjunction(parts)
        elif keyword == "anyOf":
            value = True if True in parts else None if None in parts else False
        else:
            count = sum(part is True for part in parts)
            value = False if count > 1 else None if None in parts else count == 1
        results.append(value)
    if "not" in schema:
        value = accepts_null(schema["not"], document, seen)
        results.append(None if value is None else not value)
    if "if" in schema:
        condition = accepts_null(schema["if"], document, seen)
        yes = accepts_null(schema.get("then", True), document, seen)
        no = accepts_null(schema.get("else", True), document, seen)
        results.append(yes if condition is True else no if condition is False else yes if yes == no else None)
    # Object/array/string/number-specific keywords do not apply to null.
    return conjunction(results)


def shape(schema):
    if isinstance(schema, bool):
        return "any JSON" if schema else "no value"
    if not isinstance(schema, dict):
        return "unknown"
    if "$ref" in schema:
        return schema["$ref"].rsplit("/", 1)[-1]
    for keyword in ("oneOf", "anyOf", "allOf"):
        if keyword in schema:
            return keyword + "(" + ", ".join(shape(part) for part in schema[keyword]) + ")"
    kind = schema.get("type")
    if isinstance(kind, list):
        return " | ".join(shape({**schema, "type": item}) for item in kind)
    if kind == "array":
        return "array<" + shape(schema.get("items", {})) + ">"
    if kind == "object" and isinstance(schema.get("additionalProperties"), dict):
        return "map<string, " + shape(schema["additionalProperties"]) + ">"
    if "const" in schema:
        return "const " + json.dumps(schema["const"], ensure_ascii=False)
    return kind or "unspecified"


def references(value):
    if isinstance(value, dict):
        found = {value["$ref"]} if isinstance(value.get("$ref"), str) else set()
        return found | set().union(*(references(item) for item in value.values()))
    if isinstance(value, list):
        return set().union(*(references(item) for item in value))
    return set()


def closure(value, document):
    pending = list(references(value))
    found = set()
    while pending:
        ref = pending.pop()
        if ref in found:
            continue
        found.add(ref)
        target = dereference(document, ref)
        if target is not None:
            pending.extend(references(target) - found)
    return found


def markdown(value):
    return str(value).replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;").replace("|", "\\|").replace("[", "\\[").replace("]", "\\]").replace("\n", "<br>")


def code(value, table=False):
    text = str(value).replace("\n", " ")
    if table:
        text = text.replace("|", "\\|")
    delimiter = "`"
    while delimiter in text:
        delimiter += "`"
    return delimiter + " " + text + " " + delimiter


def flag(value):
    return "unknown" if value is None else "yes" if value else "no"


def write_csv(path, rows, columns):
    with path.open("w", encoding="utf-8", newline="") as output:
        writer = csv.DictWriter(output, fieldnames=columns, extrasaction="ignore", lineterminator="\n")
        writer.writeheader()
        for row in rows:
            writer.writerow({key: json.dumps(value, ensure_ascii=False) if isinstance(value, (list, dict)) else value for key, value in row.items()})


def dump(input_path, output_path, source_ref):
    raw = input_path.read_bytes()
    document = json.loads(raw)
    if "openapi" in document:
        schemas = document["components"]["schemas"]
        schema_prefix = "#/components/schemas/"
    else:
        schemas = document["$defs"]
        schema_prefix = "#/$defs/"
    operations = []
    used_by = {name: {"request": [], "response": []} for name in schemas}
    for path, path_item in sorted(document.get("paths", {}).items()):
        for method, operation in sorted(path_item.items()):
            if method not in METHODS:
                continue
            operation_id = operation.get("operationId", f"{method.upper()} {path}")
            request = operation.get("requestBody", {})
            responses = operation.get("responses", {})
            security = operation.get("security", document.get("security", []))
            operations.append({
                "operation_id": operation_id, "method": method.upper(), "path": path,
                "security": security, "request_body_required": request.get("required", False),
                "request_refs": sorted(references(request)),
                "responses": {code: sorted(references(response)) for code, response in responses.items()},
                "parameters": path_item.get("parameters", []) + operation.get("parameters", []),
                "description": operation.get("description", operation.get("summary", "")),
                "deprecated": operation.get("deprecated", False),
                "definition": operation,
            })
            for kind, value in (("request", request), ("response", responses)):
                for ref in closure(value, document):
                    if ref.startswith(schema_prefix) and ref[len(schema_prefix):] in schemas:
                        used_by[ref[len(schema_prefix):]][kind].append(operation_id)

    fields = []

    def walk(name, schema, location, variant):
        if not isinstance(schema, dict):
            return
        required = schema.get("required", [])
        for key, definition in schema.get("properties", {}).items():
            pointer = location + "/properties/" + pointer_part(key)
            fields.append({
                "schema": name, "context": variant, "field": key, "pointer": pointer,
                "type": shape(definition), "required": key in required,
                "nullable": accepts_null(definition, document),
                "constraints": {item: definition[item] for item in BOUNDS if isinstance(definition, dict) and item in definition},
                "description": definition.get("description", "") if isinstance(definition, dict) else "",
            })
            walk(name, definition, pointer, variant + "." + key)
        for keyword in ("oneOf", "anyOf", "allOf"):
            for index, child in enumerate(schema.get(keyword, [])):
                walk(name, child, f"{location}/{keyword}/{index}", f"{variant}/{keyword}/{index}")
        if isinstance(schema.get("items"), dict):
            walk(name, schema["items"], location + "/items", variant + "[]")
        if isinstance(schema.get("additionalProperties"), dict):
            walk(name, schema["additionalProperties"], location + "/additionalProperties", variant + "{value}")

    for name, schema in sorted(schemas.items()):
        walk(name, schema, schema_prefix + pointer_part(name), name)
    summary = {
        "source_ref": source_ref, "source_file": input_path.name,
        "sha256": hashlib.sha256(raw).hexdigest(), "schemas": len(schemas),
        "object_fields": len(fields), "required_fields": sum(field["required"] for field in fields),
        "omittable_fields": sum(not field["required"] for field in fields),
        "nullable_fields": sum(field["nullable"] is True for field in fields),
        "unknown_nullability": sum(field["nullable"] is None for field in fields),
        "operations": len(operations), "paths": len(document.get("paths", {})),
        "methods": {method: sum(operation["method"] == method for operation in operations) for method in sorted({operation["method"] for operation in operations})},
    }
    output_path.mkdir(parents=True, exist_ok=True)
    inventory = {"summary": summary, "security_schemes": document.get("components", {}).get("securitySchemes", {}), "schemas": {name: {"pointer": schema_prefix + pointer_part(name), "type": shape(schema), "used_by": used_by[name], "definition": schema} for name, schema in sorted(schemas.items())}, "fields": fields, "operations": operations}
    (output_path / "inventory.json").write_text(json.dumps(inventory, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    write_csv(output_path / "fields.csv", fields, ["schema", "context", "field", "pointer", "type", "required", "nullable", "constraints", "description"])
    write_csv(output_path / "operations.csv", operations, ["operation_id", "method", "path", "security", "request_body_required", "request_refs", "responses", "parameters", "deprecated", "description"])

    models = ["# 模型与字段完整目录", "", f"生成快照：`{source_ref}`；输入 SHA-256：`{summary['sha256']}`。", "", "required 表示在所属对象 / 分支中必须出现；nullable 表示 JSON 字面量 null 是否被 schema 接受。", "不是字段使用建议或完整业务校验；分支内字段的 required 不表示所有分支都携带该字段。", "", f"{len(schemas)} schemas，{len(fields)} 个对象字段；{summary['omittable_fields']} 个可省略，{summary['nullable_fields']} 个可空。", ""]
    for name, schema in sorted(schemas.items()):
        models += [f"## {name}", "", f"类型：{code(shape(schema))}；模型可空：{flag(accepts_null(schema, document))}。", ""]
        root_bounds = {key: schema[key] for key in BOUNDS if isinstance(schema, dict) and key in schema}
        if root_bounds:
            models += ["模型约束：" + code(json.dumps(root_bounds, ensure_ascii=False, separators=(",", ":"))) + "。", ""]
        if isinstance(schema, dict) and "enum" in schema:
            models += ["枚举：" + code(json.dumps(schema["enum"], ensure_ascii=False)) + "。", ""]
        if isinstance(schema, dict) and schema.get("description"):
            models += [markdown(schema["description"]), ""]
        for kind in ("request", "response"):
            models += [f"{kind} 使用：" + (", ".join(f"`{item}`" for item in used_by[name][kind]) or "—"), ""]
        entries = [field for field in fields if field["schema"] == name]
        if entries:
            models += ["| 对象 / 分支 · 字段 | 类型 | required | nullable | 约束 | 说明 |", "| --- | --- | --- | --- | --- | --- |"]
            for field in entries:
                constraints = json.dumps(field["constraints"], ensure_ascii=False, separators=(",", ":")) if field["constraints"] else "—"
                models.append("| " + " | ".join([code(field["context"] + "." + field["field"], True), code(field["type"], True), flag(field["required"]), flag(field["nullable"]), code(constraints, True), markdown(field["description"])]) + " |")
            models.append("")
    (output_path / "models.md").write_text("\n".join(models).rstrip() + "\n", encoding="utf-8")

    endpoints = ["# HTTP 操作完整目录", "", f"生成快照：`{source_ref}`；{len(operations)} 个操作。", "", "security 是凭据类别，不代表具体网络角色权限；轮询、幂等性及业务权限须结合 handler 审查。", "", "| operationId | 方法与路径 | security | 请求 | 响应 |", "| --- | --- | --- | --- | --- |"]
    for operation in operations:
        responses = "; ".join(code + ": " + (", ".join(ref.rsplit("/", 1)[-1] for ref in refs) or "—") for code, refs in operation["responses"].items())
        endpoints.append("| " + " | ".join(map(markdown, [operation["operation_id"], operation["method"] + " " + operation["path"], json.dumps(operation["security"], ensure_ascii=False), ", ".join(ref.rsplit("/", 1)[-1] for ref in operation["request_refs"]) or "—", responses])) + " |")
    (output_path / "operations.md").write_text("\n".join(endpoints) + "\n", encoding="utf-8")
    print(json.dumps(summary, ensure_ascii=False, indent=2))


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("input", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--source-ref", required=True, help="Frozen commit / source version for this audit snapshot")
    args = parser.parse_args()
    dump(args.input, args.output, args.source_ref)
