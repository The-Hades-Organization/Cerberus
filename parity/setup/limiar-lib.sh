# Biblioteca dos setups da matriz do Limiar. `estado <nome>` desempacota no HOME do lado o estado de plugin que
# parity/limiar-estado.ts montou em parity/gerado/limiar/ (packs/ e data/) e troca @HOME@ pelo HOME do lado.
set -eu
ID=0a1b2c3d4e5f6a7b

estado() {
  tar -xzf "$(dirname "$0")/../gerado/limiar/$1.tar.gz" -C "$HOME"
  grep -rlF @HOME@ "$HOME/packs" "$HOME/data" 2>/dev/null | while IFS= read -r f; do sed -i "s|@HOME@|$HOME|g" "$f"; done
  return 0
}

# previa <variante>: a prévia do add trocada pela variante gravada em var/ (as do golden legado).
previa() { cp "$HOME/var/$1.json" "$HOME/data/plugins-staging/$ID/preview.json"; }
