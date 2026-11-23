#!/bin/sh
# Lado Rust da matriz cases-parser.toml: o cerberus do build do repo, com o erro do parser normalizado.
here=$(cd "$(dirname "$0")" && pwd)
exec "$here/normalizar-parser.sh" "$here/../target/debug/cerberus" "$@"
