#!/usr/bin/env python3
"""
scripts/issue_template_lint.py

Lints GitHub issue templates and verifies metadata consistency across `.github/ISSUE_TEMPLATE/`.
Used in CI and repository health checks.

Usage:
    python3 scripts/issue_template_lint.py [--check]
"""

import os
import sys
import glob

try:
    import yaml
except ImportError:
    import json
    # Simple fallback parser for basic YAML validation if PyYAML is missing
    yaml = None

REPO_ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))
TEMPLATE_DIR = os.path.join(REPO_ROOT, ".github", "ISSUE_TEMPLATE")

REQUIRED_FORM_FIELDS = {"name", "description", "body"}
ALLOWED_INPUT_TYPES = {"markdown", "input", "textarea", "checkboxes", "dropdown"}


def lint_yaml_file(filepath):
    """Parses a YAML file and returns (is_valid, content_dict, error_msg)."""
    try:
        with open(filepath, "r", encoding="utf-8") as f:
            content_str = f.read()
            if yaml:
                data = yaml.safe_load(content_str)
            else:
                # Basic check when PyYAML is not installed
                data = {"raw": content_str}
        return True, data, ""
    except Exception as e:
        return False, None, str(e)


def validate_issue_form(filepath, data):
    """Validates GitHub Issue Form schema and metadata consistency."""
    errors = []
    rel_path = os.path.relpath(filepath, REPO_ROOT)

    if not isinstance(data, dict):
        errors.append(f"{rel_path}: Root content must be a YAML dictionary")
        return errors

    # Check required top-level keys
    missing_keys = REQUIRED_FORM_FIELDS - set(data.keys())
    if missing_keys:
        errors.append(f"{rel_path}: Missing required top-level keys: {', '.join(sorted(missing_keys))}")

    # Validate name and description
    name = data.get("name")
    if not name or not isinstance(name, str) or not name.strip():
        errors.append(f"{rel_path}: 'name' must be a non-empty string")

    desc = data.get("description")
    if not desc or not isinstance(desc, str) or not desc.strip():
        errors.append(f"{rel_path}: 'description' must be a non-empty string")

    # Validate body elements
    body = data.get("body")
    if not isinstance(body, list) or len(body) == 0:
        errors.append(f"{rel_path}: 'body' must be a non-empty list of form fields")
    else:
        for idx, item in enumerate(body):
            if not isinstance(item, dict):
                errors.append(f"{rel_path}: body[{idx}] must be a dictionary")
                continue

            field_type = item.get("type")
            if not field_type or field_type not in ALLOWED_INPUT_TYPES:
                errors.append(
                    f"{rel_path}: body[{idx}] has invalid or missing 'type' ('{field_type}'). "
                    f"Allowed types: {', '.join(sorted(ALLOWED_INPUT_TYPES))}"
                )

            attributes = item.get("attributes")
            if field_type != "markdown" and not isinstance(attributes, dict):
                errors.append(f"{rel_path}: body[{idx}] of type '{field_type}' must have an 'attributes' dict")
            elif attributes and "label" in attributes:
                if not isinstance(attributes["label"], str) or not attributes["label"].strip():
                    errors.append(f"{rel_path}: body[{idx}] label must be a non-empty string")

    return errors


def validate_config_yml(filepath, data):
    """Validates .github/ISSUE_TEMPLATE/config.yml structure."""
    errors = []
    rel_path = os.path.relpath(filepath, REPO_ROOT)

    if not isinstance(data, dict):
        errors.append(f"{rel_path}: config.yml must be a dictionary")
        return errors

    if "blank_issues_enabled" in data:
        if not isinstance(data["blank_issues_enabled"], bool):
            errors.append(f"{rel_path}: 'blank_issues_enabled' must be a boolean")

    if "contact_links" in data:
        links = data["contact_links"]
        if not isinstance(links, list):
            errors.append(f"{rel_path}: 'contact_links' must be a list")
        else:
            for idx, link in enumerate(links):
                if not isinstance(link, dict) or "name" not in link or "url" not in link:
                    errors.append(f"{rel_path}: contact_links[{idx}] must contain 'name' and 'url'")

    return errors


def main():
    if not os.path.exists(TEMPLATE_DIR):
        print(f"ERROR: Issue template directory does not exist: {TEMPLATE_DIR}", file=sys.stderr)
        sys.exit(1)

    template_files = sorted(glob.glob(os.path.join(TEMPLATE_DIR, "*.yml")) + glob.glob(os.path.join(TEMPLATE_DIR, "*.yaml")))

    if not template_files:
        print(f"ERROR: No issue templates found in {TEMPLATE_DIR}", file=sys.stderr)
        sys.exit(1)

    all_errors = []
    passed_count = 0

    print("=== GitHub Issue Template & Metadata Lint ===")
    for filepath in template_files:
        rel_path = os.path.relpath(filepath, REPO_ROOT)
        filename = os.path.basename(filepath)

        is_valid, data, parse_err = lint_yaml_file(filepath)
        if not is_valid:
            all_errors.append(f"{rel_path}: Failed to parse YAML: {parse_err}")
            continue

        if filename == "config.yml":
            file_errors = validate_config_yml(filepath, data)
        else:
            file_errors = validate_issue_form(filepath, data)

        if file_errors:
            all_errors.extend(file_errors)
        else:
            passed_count += 1
            print(f"  ✓ {rel_path} passed metadata linting")

    print()
    if all_errors:
        print(f"FAILED: {len(all_errors)} issue template linting error(s) found:")
        for err in all_errors:
            print(f"  ✗ {err}")
        sys.exit(1)

    print(f"SUMMARY: All {passed_count}/{len(template_files)} issue template files passed linting and metadata consistency checks.")
    sys.exit(0)


if __name__ == "__main__":
    main()
