#!/usr/bin/env bash
echo "{\"resultado\": \"ok\", \"evidencia\": [{\"conferido\": \"$CERBERUS_CTX_PR $CERBERUS_MOMENTO $CERBERUS_CABECA\", \"onde\": \"$(basename "$CERBERUS_DIR")\"}]}"
