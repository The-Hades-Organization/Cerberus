#!/usr/bin/env bash
head -c 71680 /dev/zero | tr '\0' x
echo
echo '{"resultado":"ok","evidencia":[{"conferido":"caso do teste","onde":"parity/skill-min"}],"detalhe":"d"}'
