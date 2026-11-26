#!/usr/bin/env bash
# Enforces the `rust-imports` rule: absolute `crate::` paths, never `use super::...`, except the
# `use super::*;` that opens a colocated `#[cfg(test)] mod tests`.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

matches="$(grep -rnE --include='*.rs' '^[[:space:]]*(pub(\([a-z]+\))?[[:space:]]+)?use[[:space:]]+super::' apps packages |
  grep -vE 'use[[:space:]]+super::\*;' || true)"

if [[ -n "$matches" ]]; then
  echo "Use crate:: paths instead of super:: (see .ai/rules/rust-imports.md):" >&2
  echo "$matches" >&2
  exit 1
fi
