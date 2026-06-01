from __future__ import annotations

from codex_review.schema import load_schema_json, make_openai_structured_output_schema


ACTION_SCHEMA_NAMES = [
    "stage00-lifecycle-result.v1",
    "stage01-axis-findings.v1",
    "stage02-techlead-decision.v1",
    "stage03-design-inventory.v1",
    "stage03-design-clusters.v1",
    "stage03-cluster-analysis.v1",
    "stage03-design-plan.v1",
    "stage04-design-chief-decision.v1",
    "stage05-fix-agent-result.v1",
    "stage06-merged-fix.v1",
]


def iter_object_schemas(node, path=()):
    if isinstance(node, dict):
        if node.get("type") == "object" or "properties" in node:
            yield path, node
        for key, value in node.items():
            yield from iter_object_schemas(value, (*path, str(key)))
    elif isinstance(node, list):
        for index, value in enumerate(node):
            yield from iter_object_schemas(value, (*path, str(index)))


def iter_array_schemas(node, path=()):
    if isinstance(node, dict):
        if node.get("type") == "array":
            yield path, node
        for key, value in node.items():
            yield from iter_array_schemas(value, (*path, str(key)))
    elif isinstance(node, list):
        for index, value in enumerate(node):
            yield from iter_array_schemas(value, (*path, str(index)))


def iter_enum_schemas(node, path=()):
    if isinstance(node, dict):
        if "enum" in node:
            yield path, node
        for key, value in node.items():
            yield from iter_enum_schemas(value, (*path, str(key)))
    elif isinstance(node, list):
        for index, value in enumerate(node):
            yield from iter_enum_schemas(value, (*path, str(index)))


def test_openai_action_schemas_are_strict_structured_outputs():
    for name in ACTION_SCHEMA_NAMES:
        schema = make_openai_structured_output_schema(load_schema_json(name))
        for path, obj in iter_object_schemas(schema):
            assert obj.get("additionalProperties") is False, (name, path)
            properties = obj.get("properties", {})
            assert set(obj.get("required", [])) == set(properties), (name, path)
        for path, arr in iter_array_schemas(schema):
            assert "items" in arr, (name, path)
        for path, enum_schema in iter_enum_schemas(schema):
            assert "type" in enum_schema, (name, path)


def test_openai_strict_schema_keeps_defer_issue_payload_shape():
    schema = make_openai_structured_output_schema(load_schema_json("stage00-lifecycle-result.v1"))
    issue = schema["properties"]["decisions"]["items"]["properties"]["issue_request"]
    assert "null" in issue["type"]
    assert set(issue["properties"]) == {"title", "body", "root_cause_key", "labels"}


def test_openai_strict_schema_keeps_fix_policy_payload_shape():
    schema = make_openai_structured_output_schema(load_schema_json("stage04-design-chief-decision.v1"))
    policy = schema["properties"]["fix_policy"]
    assert "null" in policy["type"]
    assert {"allowed_files", "allowed_prefixes", "max_tasks", "max_patch_bytes"}.issubset(policy["properties"])
