# Agent guide

## Tests before pushing

- The full test suite runs locally before every push: `cargo test`. It must pass; never push a failing or skipped suite, and never use `--no-verify`.
- CI runs tests for outside contributors on this public repository, but that does not replace running them locally first.
