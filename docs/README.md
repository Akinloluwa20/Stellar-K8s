# Stellar-K8s Documentation

Production-grade Stellar infrastructure on Kubernetes. This directory contains all operator documentation.

## Navigation

The canonical documentation index is maintained in [`mkdocs.yml`](../mkdocs.yml) — the single navigation source for the rendered MkDocs site. Use that file to find all documentation pages organized by topic.

For a quick reference, key topic areas include:
- Getting Started, Deployment Guides, Configuration, Tutorials
- Operations & Observability, Troubleshooting
- API Reference, Security, Networking
- Development, Contributing

## Documentation Maintenance

**Owner:** Any contributor — this README is maintained alongside `mkdocs.yml`.

### Adding a new doc

1. Create your `.md` file under the appropriate `docs/` subdirectory.
2. Add a corresponding entry to `mkdocs.yml` under the relevant section.

### Updating an existing doc

- Keep links relative (e.g. `../README.md`, `./api-reference.md`).
- Run `make link-check` to catch broken links before opening a PR.

### Removing a doc

1. Delete the file.
2. Remove its entry from `mkdocs.yml`.
3. Search for cross-references: `grep -r "filename.md" docs/` and fix or remove them.

### Generating auto-derived docs

Some files are generated — do not edit them by hand:

| File | How to regenerate |
|------|------------------|
| `docs/api-reference.md` | `make generate-api-docs` |
| `completions/` | `make completions` |

Run `make health` to verify formatting, linting, tests, and docs drift in one command.
