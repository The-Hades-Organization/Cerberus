#!/usr/bin/env bash
# Monta a skill da matriz: cópia dos arquivos versionados do Hades-Skill (o TS acha o registro pela pasta do próprio
# código, então o lado TS e o Rust leem a mesma cópia via --ts e HADES_SKILL_DIR), sem o registro e o catálogo reais,
# com o parity/fixtures/skill-min por cima e os stubs de parity/stubs no PATH dos casos (loop/test/stubs).
#   montar-skill.sh <Hades-Skill> <destino> [variante]
# variante: registro extra de parity/fixtures/registro-<variante>/ (guarda quebrada para --validar).
set -euo pipefail
src=$(cd "$1" && pwd); dest=$2; var=${3:-}
here=$(cd "$(dirname "$0")" && pwd)
rm -rf "$dest"; mkdir -p "$dest"
(cd "$src" && git ls-files -z | tar -cf - --null -T -) | tar -xf - -C "$dest"
rm -rf "$dest/loop/cerberus" "$dest/kit/config.tsv" "$dest/kit/skill-release.sh"
cp -a "$here/fixtures/skill-min/." "$dest/"
cp -a "$here/stubs/." "$dest/loop/test/stubs/"
[ -n "$var" ] && cp -a "$here/fixtures/registro-$var/." "$dest/loop/cerberus/"
[ -d "$src/node_modules" ] && ln -s "$src/node_modules" "$dest/node_modules"
echo "$dest"
